// 第一关「能量校准」波形拟合内核（noocr 支线版本）。
// 主线版本的「截图识别目标曲线 → 拟合」管线已随 OCR/屏幕识别整体退役；
// 支线改为用户在结果窗口空白画布手绘目标曲线，前端采集归一化点集后
// 经 wave_fit_points 命令送入本模块拟合。
// 算法移植自 GOC解题工具11.9/analysis/solver_lib.py 的 fit_two（视频逐帧
// 端到端验证通过：双正弦拟合 R²≈0.995~0.997）：
//   双正弦（基波+二次谐波）网格搜索 + 线性最小二乘拟合；
//   基波参数逆映射回四个滑块（0~100）：振幅/频率/相位/偏移。
// 归一化约定与主线 curve_points 完全一致：x∈[0,1]（左→右）、
// y∈[-1,1]（向上为正，游戏画布同向）。

use serde_json::{json, Value};

// 单飞保护沿用主线语义：并发拟合虽无截图缓存写坏风险，但毫秒级纯数学
// 无需并发上限，此处不加锁；命令为同步直接返回。

// ---------------- 对外入口 ----------------

// 手绘点拟合主入口：入参为归一化曲线点（x∈[0,1]，y∈[-1,1] 向上为正）。
// 成功出参与主线 solve_image 的 payload 结构完全一致：
//   { ok, msg, sliders:{amp,freq,phase,off}, curve:{a1,f1,p1,off,a2,f2,p2} }
// 前端 waveCurvePath 零改动消费 curve 参数绘制预览。
pub fn fit_points(points: Vec<(f32, f32)>) -> Value {
    // 点数守卫：太少无法约束 5 参数双正弦（基波+谐波+偏移）
    if points.len() < 60 {
        return payload(false, None, "画线点太少，请沿画布完整画出目标曲线（从最左画到最右）", None);
    }
    // 重采样：手绘点 x 分布不均匀（拐点处密集、直线段稀疏），直接喂网格
    // 搜索会被密集段带偏频率粗搜；按 x 升序线性插值等距重采样到 400 点
    let pts = resample(&points, 400);
    let fit = fit_two(&pts);
    // 拟合质量校验阈值与主线 solve_image 候选循环一致（R²≥0.5）
    if fit.r2 < 0.5 {
        return payload(
            false,
            None,
            &format!("曲线拟合质量不足（R²={:.3}），请尽量贴合游戏内目标曲线重画", fit.r2),
            None,
        );
    }
    let (amp, freq, phase, off) = to_sliders(&fit);
    info!(
        "[wave] 手绘拟合 R²={:.4} f1={:.3} f2={:.3} A1={:.3} → 滑块 振幅{} 频率{} 相位{} 偏移{}",
        fit.r2, fit.f1, fit.f2, fit.a1, amp, freq, phase, off
    );
    payload(
        true,
        Some(json!({ "amp": amp, "freq": freq, "phase": phase, "off": off })),
        "",
        Some(json!({
            "a1": fit.a1, "f1": fit.f1, "p1": fit.p1, "off": fit.off,
            "a2": fit.a2, "f2": fit.f2, "p2": fit.p2,
        })),
    )
}

fn payload(ok: bool, sliders: Option<Value>, msg: &str, curve: Option<Value>) -> Value {
    json!({ "ok": ok, "msg": msg, "sliders": sliders, "curve": curve })
}

use log::info;

// 等距重采样：先按 x 升序排序，再在 n 个等距 x 位置线性插值取 y。
// 输入点 x 需覆盖 [0,1] 全程（前端从画布最左画到最右，天然满足）；
// 首尾点不足 x=0/1 时以实际首尾为界外推 clamp，避免 NaN。
fn resample(points: &[(f32, f32)], n: usize) -> Vec<(f32, f32)> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    sorted.dedup_by(|a, b| {
        // 相邻 x 几乎相同的点取均值（手绘停顿会造成竖直堆叠）
        if (a.0 - b.0).abs() < 1e-4 {
            b.1 = (a.1 + b.1) / 2.0;
            b.0 = a.0;
            true
        } else {
            false
        }
    });
    let m = sorted.len();
    if m == 0 {
        return Vec::new();
    }
    if m == 1 {
        return vec![sorted[0]; n];
    }
    let (x0, x1) = (sorted[0].0, sorted[m - 1].0);
    let span = (x1 - x0).max(1e-6);
    let mut out = Vec::with_capacity(n);
    let mut j = 0usize; // 当前段右端点索引（sorted[j-1]..sorted[j]）
    for k in 0..n {
        let x = x0 + span * (k as f32 / (n - 1) as f32);
        while j < m && sorted[j].0 < x {
            j += 1;
        }
        let (ya, xa) = if j == 0 {
            (sorted[0].1, sorted[0].0)
        } else {
            (sorted[j - 1].1, sorted[j - 1].0)
        };
        let (yb, xb) = if j >= m {
            (sorted[m - 1].1, sorted[m - 1].0)
        } else {
            (sorted[j].1, sorted[j].0)
        };
        let t = if (xb - xa).abs() < 1e-6 {
            0.0
        } else {
            ((x - xa) / (xb - xa)).clamp(0.0, 1.0)
        };
        out.push((x, ya + (yb - ya) * t));
    }
    out
}

// ---------------- 线性最小二乘（列 = [1, sin/cos(2πf·x) × 频率数]） ----------------

// 返回 (sse, coef)；sse = Σy² - coefᵀXᵀy（正规方程精确解）
fn solve_lin(pts: &[(f32, f32)], freqs: &[f32]) -> (f32, Vec<f32>) {
    let k = 1 + freqs.len() * 2;
    let mut xt_x = vec![vec![0f32; k]; k];
    let mut xty = vec![0f32; k];
    let mut yy = 0f32;
    let ws: Vec<f32> = freqs
        .iter()
        .map(|f| std::f32::consts::TAU * f)
        .collect();
    for &(x, y) in pts {
        let mut cols = Vec::with_capacity(k);
        cols.push(1.0);
        for w in ws.iter() {
            cols.push((w * x).sin());
            cols.push((w * x).cos());
        }
        for i in 0..k {
            xty[i] += cols[i] * y;
            for j in i..k {
                xt_x[i][j] += cols[i] * cols[j];
            }
        }
        yy += y * y;
    }
    for i in 0..k {
        for j in i..k {
            xt_x[j][i] = xt_x[i][j];
        }
    }
    let coef = gauss_solve(&xt_x, &xty);
    let mut sse = yy;
    for i in 0..k {
        sse -= coef[i] * xty[i];
    }
    (sse.max(0.0), coef)
}

// 通用高斯消元（部分主元，n ≤ 5）
fn gauss_solve(a: &[Vec<f32>], b: &[f32]) -> Vec<f32> {
    let n = b.len();
    let mut m = vec![vec![0f32; n + 1]; n];
    for i in 0..n {
        m[i][..n].copy_from_slice(&a[i]);
        m[i][n] = b[i];
    }
    for col in 0..n {
        let mut piv = col;
        for r in col + 1..n {
            if m[r][col].abs() > m[piv][col].abs() {
                piv = r;
            }
        }
        m.swap(col, piv);
        let d = m[col][col];
        if d.abs() < 1e-12 {
            continue;
        }
        for r in 0..n {
            if r != col && m[r][col] != 0.0 {
                let f = m[r][col] / d;
                for c in col..n + 1 {
                    m[r][c] -= f * m[col][c];
                }
            }
        }
    }
    (0..n)
        .map(|i| if m[i][i].abs() < 1e-12 { 0.0 } else { m[i][n] / m[i][i] })
        .collect()
}

fn sse_of(pts: &[(f32, f32)], f1: f32, f2: f32) -> f32 {
    solve_lin(pts, &[f1, f2]).0
}

// 单频拟合 SSE：搜基波时用（双频率占位会让设计矩阵近奇异，高斯消元解发散，
// 峰位被带偏；单频 3 列方程组条件数良好）
fn sse1(pts: &[(f32, f32)], f: f32) -> f32 {
    solve_lin(pts, &[f]).0
}

// 频率细化：3 轮，每轮在 ±span 内以 span/10 步长搜峰，span 每轮 /4。
// other = Some(o) 时按 双频率(o 固定) 评估（联合微调阶段），None 时按单频评估
fn refine_f(pts: &[(f32, f32)], f: f32, span: f32, other: Option<f32>) -> f32 {
    let mut f = f;
    let mut span = span;
    for _ in 0..3 {
        let lo = (f - span).max(0.05);
        let step = span / 10.0;
        let mut best = (f32::INFINITY, f);
        let mut v = lo;
        while v <= f + span {
            let s = match other {
                Some(o) => sse_of(pts, v, o),
                None => sse1(pts, v),
            };
            if s < best.0 {
                best = (s, v);
            }
            v += step;
        }
        f = best.1;
        span /= 4.0;
    }
    f
}

// fit_two：基波粗搜 0.3~3.3 → 细化 → 二次谐波在 [1.5f1, min(9.5, 3.3f1)] 粗搜 →
// 细化 → 两轮联合微调（全部移植自 solver_lib.fit_two）
fn fit_two(pts: &[(f32, f32)]) -> Fit {
    let mut best1 = (f32::INFINITY, 0.8f32);
    let mut f0 = 0.3f32;
    while f0 <= 3.31 {
        let s = sse1(pts, f0);
        if s < best1.0 {
            best1 = (s, f0);
        }
        f0 += 0.1;
    }
    let mut f1 = refine_f(pts, best1.1, 0.06, None);

    let lo = (1.5 * f1).max(0.3);
    let hi = (3.3 * f1).min(9.5);
    let mut best2 = (f32::INFINITY, f1 * 2.0);
    let mut f2 = lo;
    while f2 <= hi {
        if (f2 - f1).abs() >= 0.15 {
            let s = sse_of(pts, f1, f2);
            if s < best2.0 {
                best2 = (s, f2);
            }
        }
        f2 += 0.05;
    }
    let mut f2 = refine_f(pts, best2.1, 0.04, Some(f1));

    // 坐标下降式联合微调：交替固定另一频率细化当前频率
    for _ in 0..2 {
        let f1n = refine_f(pts, f1, 0.012, Some(f2));
        let f2n = refine_f(pts, f2, 0.012, Some(f1n));
        if sse_of(pts, f1n, f2n) < sse_of(pts, f1, f2) {
            f1 = f1n;
            f2 = f2n;
        }
    }

    let (_, coef) = solve_lin(pts, &[f1, f2]);
    let yy: f32 = pts.iter().map(|&(_, y)| y * y).sum();
    let sse = sse_of(pts, f1, f2);
    let a1 = coef[1].hypot(coef[2]);
    let mut p1 = coef[2].atan2(coef[1]);
    if p1 < 0.0 {
        p1 += std::f32::consts::TAU;
    }
    // 二次谐波参数（coef = [off, sin1, cos1, sin2, cos2]）：预览曲线与游戏画布
    // 完全同构必需，只推基波会缺谐波波纹
    let a2 = coef[3].hypot(coef[4]);
    let p2 = coef[4].atan2(coef[3]);
    let off = coef[0];
    let r2 = if yy > 0.0 { 1.0 - sse / yy } else { 0.0 };
    Fit { f1, f2, a1, p1, off, r2, a2, p2 }
}

struct Fit {
    f1: f32,
    f2: f32,
    a1: f32,
    p1: f32,
    off: f32,
    r2: f32,
    a2: f32,
    p2: f32,
}

// 滑块逆映射（11.9 实测标定；偏移滑块左=上，方向已含在公式内）
fn to_sliders(fit: &Fit) -> (i64, i64, i64, i64) {
    let amp = ((fit.a1 - 0.2) / 0.4 * 100.0).round() as i64;
    let freq = ((fit.f1 - 0.3) / 2.7 * 100.0).round() as i64;
    let phase = (fit.p1 / std::f32::consts::TAU * 100.0).round() as i64;
    let off = ((0.15 - fit.off) / 0.36 * 100.0).round() as i64;
    let clamp = |v: i64| v.clamp(0, 100);
    (clamp(amp), clamp(freq), clamp(phase), clamp(off))
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    // 合成已知参数双正弦采样点 → fit_points → payload 结构与滑块断言
    #[test]
    fn wave_fit_points_payload() {
        let (a1, f1, p1, off, a2, f2, p2) =
            (0.35f32, 1.4f32, 1.1f32, -0.05f32, 0.08f32, 3.9f32, 2.2f32);
        let pts: Vec<(f32, f32)> = (0..300)
            .map(|i| {
                let x = i as f32 / 299.0;
                let y = off
                    + a1 * (std::f32::consts::TAU * f1 * x + p1).sin()
                    + a2 * (std::f32::consts::TAU * f2 * x + p2).sin();
                (x, y)
            })
            .collect();
        let v = fit_points(pts);
        println!("fit payload: {}", v);
        assert!(v["ok"].as_bool().unwrap(), "应拟合成功: {}", v["msg"]);
        let s = v["sliders"].as_object().expect("应有滑块对象");
        for k in ["amp", "freq", "phase", "off"] {
            let val = s[k].as_i64().expect("滑块应为整数");
            assert!((0..=100).contains(&val), "滑块 {} 越界: {}", k, val);
        }
        let c = v["curve"].as_object().expect("应有曲线参数对象");
        for k in ["a1", "f1", "p1", "off", "a2", "f2", "p2"] {
            assert!(c[k].is_number(), "curve.{} 缺失", k);
        }
        // 振幅/频率滑块应接近理论值（与主线 synthetic 同款容差）
        assert!((s["amp"].as_i64().unwrap() - 37).abs() <= 6, "振幅滑块偏差过大");
        assert!((s["freq"].as_i64().unwrap() - 41).abs() <= 3, "频率滑块偏差过大");
    }

    // 非均匀 x 分布（模拟手绘：拐点密集、中部稀疏+乱序输入）应经重采样后还原频率
    #[test]
    fn wave_fit_points_resample() {
        let (a1, f1, p1, off) = (0.32f32, 1.7f32, 0.6f32, 0.0f32);
        // 乱序 + 非均匀 x：前半段密（步长0.002）、后半段疏（步长0.02）
        let mut pts: Vec<(f32, f32)> = Vec::new();
        let mut xs: Vec<f32> = Vec::new();
        let mut x = 0.0f32;
        while x <= 0.5 {
            xs.push(x);
            x += 0.002;
        }
        x = 0.52;
        while x <= 1.0 {
            xs.push(x);
            x += 0.02;
        }
        for &x in &xs {
            let y = off + a1 * (std::f32::consts::TAU * f1 * x + p1).sin();
            pts.push((x, y));
        }
        pts.reverse(); // 乱序输入
        let v = fit_points(pts);
        assert!(v["ok"].as_bool().unwrap(), "非均匀点应拟合成功: {}", v["msg"]);
        let freq = v["sliders"]["freq"].as_i64().unwrap();
        let expect = ((f1 - 0.3) / 2.7 * 100.0).round() as i64;
        assert!((freq - expect).abs() <= 4, "频率滑块偏差过大: {} vs {}", freq, expect);
    }

    // 点数不足应返回失败 payload 而非 panic
    #[test]
    fn wave_fit_points_too_few() {
        let pts: Vec<(f32, f32)> = (0..10)
            .map(|i| (i as f32 / 9.0, 0.1 * (i as f32).sin()))
            .collect();
        let v = fit_points(pts);
        assert!(!v["ok"].as_bool().unwrap(), "点太少应失败");
        assert!(!v["msg"].as_str().unwrap().is_empty());
        assert!(v["sliders"].is_null());
    }

    // 主线遗留回归：合成信号经 fit_two 直接拟合（不加重采样）应还原参数
    #[test]
    fn wave_synthetic_roundtrip() {
        let n = 500;
        let (a1, f1, p1, off, a2, f2, p2) =
            (0.35f32, 1.4f32, 1.1f32, -0.05f32, 0.08f32, 3.9f32, 2.2f32);
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let x = i as f32 / (n - 1) as f32;
                let y = off
                    + a1 * (std::f32::consts::TAU * f1 * x + p1).sin()
                    + a2 * (std::f32::consts::TAU * f2 * x + p2).sin();
                (x, y)
            })
            .collect();
        let fit = fit_two(&pts);
        println!(
            "synth: f1={:.3}(期望1.4) f2={:.3}(期望3.9) A1={:.3}(期望0.35) p1={:.3}(期望1.1) off={:.3}(期望-0.05)",
            fit.f1, fit.f2, fit.a1, fit.p1, fit.off
        );
        assert!((fit.f1 - f1).abs() < 0.03, "基波频率偏差过大");
        assert!((fit.a1 - a1).abs() < 0.02, "基波振幅偏差过大");
        assert!((fit.off - off).abs() < 0.02, "偏移偏差过大");
        let (amp, freq, _phase, _off) = to_sliders(&fit);
        println!("synth 滑块: amp={} freq={}", amp, freq);
        assert!((amp - 37).abs() <= 6, "振幅滑块偏差过大: {}", amp);
        assert!((freq - 41).abs() <= 3, "频率滑块偏差过大: {}", freq);
    }
}

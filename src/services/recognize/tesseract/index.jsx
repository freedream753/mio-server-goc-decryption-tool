import Tesseract from 'tesseract.js';

// GOC 题目为深蓝色密文压在深色半透明背景上，直接 OCR 识别率极低（实测全对率 12%）。
// 预处理管线（两张实拍图实测全对率 100%）：
//   1. 提取蓝主导通道 d = B - max(R, G)：密文字像素 d 约 100~200，背景 < 80
//   2. 放大 4 倍（密文仅约 14px 高，过小）
//   3. 以固定阈值 80 二值化，得到白底黑字图
//   4. Tesseract 限定大写白名单 + psm 6，识别后过滤噪声 token
const BLUE_THRESHOLD = 80;
const UPSCALE = 4;
const LANG_PATH = 'https://pub-f6afb74f13c64cd89561b4714dca1c27.r2.dev';

// 预处理：蓝主导通道提取 + 放大 + 二值化，返回白底黑字 dataURL
async function preprocess(base64) {
    const image = await new Promise((resolve, reject) => {
        const img = new Image();
        img.onload = () => resolve(img);
        img.onerror = reject;
        img.src = 'data:image/png;base64,' + base64;
    });
    const { width, height } = image;
    // 小图画布：像素灰度值 = 蓝主导强度 d（clamp 到 0~255），保留渐变信息供放大后二值化
    const small = document.createElement('canvas');
    small.width = width;
    small.height = height;
    const sctx = small.getContext('2d');
    sctx.drawImage(image, 0, 0);
    const src = sctx.getImageData(0, 0, width, height);
    const gray = sctx.createImageData(width, height);
    for (let i = 0; i < width * height; i++) {
        const r = src.data[i * 4];
        const g = src.data[i * 4 + 1];
        const b = src.data[i * 4 + 2];
        const d = Math.min(255, Math.max(0, b - Math.max(r, g)));
        gray.data[i * 4] = gray.data[i * 4 + 1] = gray.data[i * 4 + 2] = d;
        gray.data[i * 4 + 3] = 255;
    }
    sctx.putImageData(gray, 0, 0);
    // 放大后二值化：d > 阈值 → 黑（密文），否则 → 白（背景）
    const big = document.createElement('canvas');
    big.width = width * UPSCALE;
    big.height = height * UPSCALE;
    const bctx = big.getContext('2d');
    bctx.imageSmoothingEnabled = true;
    bctx.drawImage(small, 0, 0, big.width, big.height);
    const scaled = bctx.getImageData(0, 0, big.width, big.height);
    const bin = bctx.createImageData(big.width, big.height);
    for (let i = 0; i < big.width * big.height; i++) {
        const v = scaled.data[i * 4] > BLUE_THRESHOLD ? 0 : 255;
        bin.data[i * 4] = bin.data[i * 4 + 1] = bin.data[i * 4 + 2] = v;
        bin.data[i * 4 + 3] = 255;
    }
    bctx.putImageData(bin, 0, 0);
    return big.toDataURL('image/png');
}

// 密文恒为同一行内 3~8 位的大写单词；框选整窗时蓝色标题可能被误识别为噪声行。
// 取密文 token 最多的行，并列时取更靠下的行（密文行恒在标题行下方），无有效行时回退原文
function filterCipherWords(data) {
    const lines = (data.lines ?? [])
        .map((line) => ({
            y: line.bbox?.y0 ?? 0,
            words: line.text.toUpperCase().match(/(?<![A-Z])[A-Z]{3,8}(?![A-Z])/g) ?? [],
        }))
        .filter((line) => line.words.length > 0);
    const best = lines.sort((a, b) => a.words.length - b.words.length || a.y - b.y).pop();
    return best ? best.words.join(' ') : data.text.trim();
}

// GOC 题目内容为纯英文，固定使用英文模型识别
export async function recognize(base64) {
    const worker = await Tesseract.createWorker('eng', 1, {
        workerPath: '/worker.min.js',
        corePath: '/tesseract-core-simd-lstm.wasm.js',
        langPath: LANG_PATH,
        logger: () => {},
    });
    try {
        await worker.setParameters({
            tessedit_char_whitelist: 'ABCDEFGHIJKLMNOPQRSTUVWXYZ',
            tessedit_pageseg_mode: '6',
            user_defined_dpi: '300',
        });
        const prepared = await preprocess(base64);
        const { data } = await worker.recognize(prepared);
        return filterCipherWords(data);
    } finally {
        await worker.terminate();
    }
}

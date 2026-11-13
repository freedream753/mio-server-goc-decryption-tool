// 裁切模式指示层（v1.2.7）：窗口由 Rust 创建（label="crop"，透明置顶、
// set_ignore_cursor_events 点击穿透），窗口比裁切区四周各外扩 14px。
// 这里只画两样东西：贴着裁切区边缘的橙色边框 + 左上角「裁切模式」角标，
// 让用户随时知道校准/西蒙/解密只识别框内画面。
const PAD = 14;

export default function CropOverlay() {
    return (
        <div style={{ position: 'fixed', inset: 0, pointerEvents: 'none', overflow: 'hidden' }}>
            {/* 裁切区边框：与 result 窗口激活按钮同色（橙），阴影加深可见性 */}
            <div
                style={{
                    position: 'absolute',
                    top: PAD,
                    left: PAD,
                    right: PAD,
                    bottom: PAD,
                    border: '2px solid #ffb35c',
                    borderRadius: 4,
                    boxShadow: '0 0 0 1px rgba(0,0,0,0.5), 0 0 14px rgba(255,179,92,0.6)',
                }}
            />
            {/* 角标：骑在边框上沿外侧，不遮框内画面 */}
            <div
                style={{
                    position: 'absolute',
                    top: 0,
                    left: 0,
                    padding: '3px 12px',
                    background: '#ffb35c',
                    color: '#1c1206',
                    fontSize: 13,
                    fontWeight: 700,
                    lineHeight: 1.4,
                    borderRadius: '0 0 8px 0',
                    boxShadow: '0 1px 4px rgba(0,0,0,0.5)',
                }}
            >
                裁切模式 · 识别仅限框内
            </div>
        </div>
    );
}

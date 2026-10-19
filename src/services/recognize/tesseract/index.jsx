import Tesseract from 'tesseract.js';

// GOC 题目内容为纯英文，固定使用英文模型识别
export async function recognize(base64) {
    const {
        data: { text },
    } = await Tesseract.recognize('data:image/png;base64,' + base64, 'eng', {
        workerPath: '/worker.min.js',
        corePath: '/tesseract-core-simd-lstm.wasm.js',
        langPath: 'https://pub-f6afb74f13c64cd89561b4714dca1c27.r2.dev',
    });
    return text.trim();
}

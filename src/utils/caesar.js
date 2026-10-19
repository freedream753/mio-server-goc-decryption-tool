// 凯撒密码：固定位移 3 位解密（经典凯撒，密文每个字母向前移 3 位还原明文）

// 判断文本是否包含英文字母
export function hasEnglishLetters(text) {
    return /[a-zA-Z]/.test(text);
}

// 将字母向后移动 shift 位（shift 可为负），非字母字符保持原样
export function caesarShift(text, shift) {
    const k = ((shift % 26) + 26) % 26;
    return text.replace(/[a-zA-Z]/g, (ch) => {
        const base = ch <= 'Z' ? 65 : 97;
        return String.fromCharCode(((ch.charCodeAt(0) - base + k) % 26) + base);
    });
}

// 固定位移常量
export const CAESAR_SHIFT = 3;

// 凯撒密码位移 3 解密：明文 = 密文向前移 3 位（等价向后移 23 位）
export function crackCaesar(text) {
    return {
        shift: CAESAR_SHIFT,
        plaintext: caesarShift(text, 26 - CAESAR_SHIFT),
    };
}
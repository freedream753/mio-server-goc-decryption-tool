import { initReactI18next } from 'react-i18next';
import i18n from 'i18next';
import zh_CN from './locales/zh_CN.json';
import en_US from './locales/en_US.json';

// v1.4.0 精简：仅保留简体中文与英文（其余语言包已删除）。
// 缺失 key 按 default → en 回退；初始语言 zh_cn，避免 config 读取完成前 t() 显示 key 名。
i18n.use(initReactI18next).init({
    lng: 'zh_cn',
    fallbackLng: { default: ['en'] },
    debug: false,
    interpolation: {
        escapeValue: false,
    },
    resources: {
        en: en_US,
        zh_cn: zh_CN,
    },
});

export default i18n;

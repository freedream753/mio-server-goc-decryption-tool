export const debounce = (fn, delay = 500) => {
    let timer = null;
    let lastArgs = null;
    const debounced = (...args) => {
        lastArgs = args;
        timer && clearTimeout(timer);
        timer = setTimeout(() => {
            timer = null;
            fn(...args);
        }, delay);
    };
    // 立即执行仍在防抖等待中的调用（用于窗口销毁前把未落盘的配置写入 store）
    debounced.flush = () => {
        if (timer !== null) {
            clearTimeout(timer);
            timer = null;
            fn(...lastArgs);
        }
    };
    return debounced;
};

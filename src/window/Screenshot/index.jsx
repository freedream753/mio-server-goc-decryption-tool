import React, { useEffect, useState, useRef } from 'react';
import { appCacheDir, join } from '@tauri-apps/api/path';
import { currentMonitor } from '@tauri-apps/api/window';
import { convertFileSrc } from '@tauri-apps/api/tauri';
import { appWindow } from '@tauri-apps/api/window';
import { emit, listen } from '@tauri-apps/api/event';
const warn = console.warn;
import { invoke } from '@tauri-apps/api';

export default function Screenshot() {
    const [imgurl, setImgurl] = useState('');
    const [isMoved, setIsMoved] = useState(false);
    const [isDown, setIsDown] = useState(false);
    const [mouseDownX, setMouseDownX] = useState(0);
    const [mouseDownY, setMouseDownY] = useState(0);
    const [mouseMoveX, setMouseMoveX] = useState(0);
    const [mouseMoveY, setMouseMoveY] = useState(0);

    const imgRef = useRef();

    useEffect(() => {
        const capture = () => {
            currentMonitor().then((monitor) => {
                const position = monitor.position;
                invoke('screenshot', { x: position.x, y: position.y })
                    .then(() => {
                        appCacheDir().then((appCacheDirPath) => {
                            join(appCacheDirPath, 'pot_screenshot.png').then((filePath) => {
                                // 追加时间戳打破同一 URL 的缓存：截图窗口被复用时，
                                // 只有 src 变化才会重新加载新截图并触发 onLoad -> show()
                                setImgurl(`${convertFileSrc(filePath)}?t=${Date.now()}`);
                            });
                        });
                    })
                    .catch((e) => {
                        // 截图失败时关闭隐藏窗口，避免残留僵尸窗口
                        console.error(e);
                        void appWindow.close();
                    });
            });
        };
        capture();
        // 复用已存在的截图窗口时（窗口未被销毁），Rust 会发送该事件要求重新截屏
        const unlisten = listen('screenshot_request', () => capture());
        return () => {
            unlisten.then((f) => f());
        };
    }, []);

    return (
        <>
            <img
                ref={imgRef}
                className='fixed top-0 left-0 w-full select-none'
                src={imgurl}
                draggable={false}
                onLoad={() => {
                    if (imgurl !== '' && imgRef.current.complete) {
                        void appWindow.show();
                        void appWindow.setFocus();
                        void appWindow.setResizable(false);
                    }
                }}
            />
            <div
                className={`fixed bg-[#2080f020] border border-solid border-sky-500 ${!isMoved && 'hidden'}`}
                style={{
                    top: Math.min(mouseDownY, mouseMoveY),
                    left: Math.min(mouseDownX, mouseMoveX),
                    bottom: screen.height - Math.max(mouseDownY, mouseMoveY),
                    right: screen.width - Math.max(mouseDownX, mouseMoveX),
                }}
            />
            <div
                className='fixed top-0 left-0 bottom-0 right-0 cursor-crosshair select-none'
                onMouseDown={(e) => {
                    if (e.buttons === 1) {
                        setIsDown(true);
                        setMouseDownX(e.clientX);
                        setMouseDownY(e.clientY);
                    } else {
                        void appWindow.close();
                    }
                }}
                onMouseMove={(e) => {
                    if (isDown) {
                        setIsMoved(true);
                        setMouseMoveX(e.clientX);
                        setMouseMoveY(e.clientY);
                    }
                }}
                onMouseUp={async (e) => {
                    appWindow.hide();
                    setIsDown(false);
                    setIsMoved(false);
                    const imgWidth = imgRef.current.naturalWidth;
                    const dpi = imgWidth / screen.width;
                    const left = Math.floor(Math.min(mouseDownX, e.clientX) * dpi);
                    const top = Math.floor(Math.min(mouseDownY, e.clientY) * dpi);
                    const right = Math.floor(Math.max(mouseDownX, e.clientX) * dpi);
                    const bottom = Math.floor(Math.max(mouseDownY, e.clientY) * dpi);
                    const width = right - left;
                    const height = bottom - top;
                    if (width <= 0 || height <= 0) {
                        warn('Screenshot area is too small');
                        await appWindow.close();
                    } else {
                        await invoke('cut_image', { left, top, width, height });
                        await emit('success');
                        await appWindow.close();
                    }
                }}
            />
        </>
    );
}

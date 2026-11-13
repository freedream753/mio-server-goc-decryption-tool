import { VscChromeClose, VscChromeMinimize, VscChromeMaximize, VscChromeRestore } from 'react-icons/vsc';
import React, { useEffect, useState } from 'react';
import { appWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { Button } from '@nextui-org/react';

import { osType } from '../../utils/env';
import './style.css';

export default function WindowControl() {
    const [isMax, setIsMax] = useState(false);

    useEffect(() => {
        // 挂载时先查询一次当前最大化状态，避免按钮图标初始显示错误
        appWindow.isMaximized().then(setIsMax);
        // 保存取消监听函数，卸载时清理，避免监听泄漏
        const unlisten = listen('tauri://resize', async () => {
            if (await appWindow.isMaximized()) {
                setIsMax(true);
            } else {
                setIsMax(false);
            }
        });
        return () => {
            unlisten.then((f) => {
                f();
            });
        };
    }, []);

    return (
        <div>
            <Button
                isIconOnly
                variant='light'
                className='w-[35px] h-[35px] rounded-none'
                onPress={() => appWindow.minimize()}
            >
                <VscChromeMinimize className='text-[16px]' />
            </Button>
            <Button
                isIconOnly
                variant='light'
                className='w-[35px] h-[35px] rounded-none'
                onPress={() => {
                    if (isMax) {
                        appWindow.unmaximize();
                    } else {
                        appWindow.maximize();
                    }
                }}
            >
                {isMax ? <VscChromeRestore className='text-[16px]' /> : <VscChromeMaximize className='text-[16px]' />}
            </Button>
            <Button
                isIconOnly
                variant='light'
                className={`w-[35px] h-[35px] rounded-none close-button ${osType === 'Linux' && 'rounded-tr-[10px]'}`}
                onPress={() => appWindow.close()}
            >
                <VscChromeClose className='text-[16px]' />
            </Button>
        </div>
    );
}

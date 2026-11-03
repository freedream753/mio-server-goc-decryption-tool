import toast, { Toaster } from 'react-hot-toast';
import { useTranslation } from 'react-i18next';
import { CardBody } from '@nextui-org/react';
import { Button } from '@nextui-org/react';
import { Input } from '@nextui-org/react';
import { Card } from '@nextui-org/react';
import React, { useState } from 'react';

import { useConfig } from '../../../../hooks/useConfig';
import { useToastStyle } from '../../../../hooks';
import { osType } from '../../../../utils/env';
import { invoke } from '@tauri-apps/api';

const keyMap = {
    Backquote: '`',
    Backslash: '\\',
    BracketLeft: '[',
    BracketRight: ']',
    Comma: ',',
    Equal: '=',
    Minus: '-',
    Plus: 'PLUS',
    Period: '.',
    Quote: "'",
    Semicolon: ';',
    Slash: '/',
    Backspace: 'Backspace',
    CapsLock: 'Capslock',
    ContextMenu: 'Contextmenu',
    Space: 'Space',
    Tab: 'Tab',
    Convert: 'Convert',
    Delete: 'Delete',
    End: 'End',
    Help: 'Help',
    Home: 'Home',
    PageDown: 'Pagedown',
    PageUp: 'Pageup',
    Escape: 'Esc',
    PrintScreen: 'Printscreen',
    ScrollLock: 'Scrolllock',
    Pause: 'Pause',
    Insert: 'Insert',
    Suspend: 'Suspend',
};

export default function Hotkey() {
    const [ocrRecognize, setOcrRecognize] = useConfig('hotkey_ocr_recognize', 'Alt+C');
    // 输入框为草稿态：编辑期间不注销旧热键、不写 store；
    // 仅点击「确定」且后端注册成功后才落盘，失败保留原热键
    const [draft, setDraft] = useState('');

    const { t } = useTranslation();
    const toastStyle = useToastStyle();

    function keyDown(e) {
        e.preventDefault();
        if (e.keyCode === 8) {
            setDraft('');
        } else {
            let newValue = '';
            if (e.ctrlKey) {
                newValue = 'Ctrl';
            }
            if (e.shiftKey) {
                newValue = `${newValue}${newValue.length > 0 ? '+' : ''}Shift`;
            }
            if (e.metaKey) {
                newValue = `${newValue}${newValue.length > 0 ? '+' : ''}${osType === 'Darwin' ? 'Command' : 'Super'}`;
            }
            if (e.altKey) {
                newValue = `${newValue}${newValue.length > 0 ? '+' : ''}Alt`;
            }
            let code = e.code;
            if (code.startsWith('Key')) {
                code = code.substring(3);
            } else if (code.startsWith('Digit')) {
                code = code.substring(5);
            } else if (code.startsWith('Numpad')) {
                code = 'Num' + code.substring(6);
            } else if (code.startsWith('Arrow')) {
                code = code.substring(5);
            } else if (code.startsWith('Intl')) {
                code = code.substring(4);
            } else if (/F\d+/.test(code)) {
            } else if (keyMap[code] !== undefined) {
                code = keyMap[code];
            } else {
                code = '';
            }
            setDraft(`${newValue}${newValue.length > 0 && code.length > 0 ? '+' : ''}${code}`);
        }
    }

    // 点「确定」才提交：后端先注销旧键再注册新键，失败自动回滚旧键并报错
    function applyDraft() {
        if (!draft || draft === ocrRecognize) {
            setDraft('');
            return;
        }
        invoke('update_shortcut', {
            name: 'hotkey_ocr_recognize',
            shortcut: draft,
        }).then(
            () => {
                toast.success(t('config.hotkey.success'), { style: toastStyle });
                // Rust 侧已落盘；同步前端 useConfig 状态使输入框立即显示新热键
                setOcrRecognize(draft, true);
                setDraft('');
            },
            (e) => {
                toast.error(e, { style: toastStyle });
            }
        );
    }

    return (
        <Card>
            <Toaster />
            <CardBody>
                <div className='config-item'>
                    <h3 className='my-auto'>{t('config.hotkey.ocr_recognize')}</h3>
                    {ocrRecognize !== null && (
                        <Input
                            type='hotkey'
                            variant='bordered'
                            value={draft || ocrRecognize}
                            label={t('config.hotkey.set_hotkey')}
                            className='max-w-[50%]'
                            onKeyDown={keyDown}
                            onFocus={() => {
                                // 聚焦只清空草稿重新捕获，不影响已生效热键
                                setDraft('');
                            }}
                            endContent={
                                <Button
                                    size='sm'
                                    variant='flat'
                                    className={`${draft === '' && 'hidden'}`}
                                    onPress={applyDraft}
                                >
                                    {t('common.ok')}
                                </Button>
                            }
                        />
                    )}
                </div>
            </CardBody>
        </Card>
    );
}

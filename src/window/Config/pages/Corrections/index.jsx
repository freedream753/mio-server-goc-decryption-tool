// 自学习词库设置页：维护密文 -> 答案映射（answers.json）
// 识别管线在词表未命中时优先参考该词库（命中显示 ★）
import { invoke } from '@tauri-apps/api';
import { Button, Card, Input } from '@nextui-org/react';
import React, { useEffect, useState } from 'react';

export default function Corrections() {
    const [items, setItems] = useState([]);
    const [cipher, setCipher] = useState('');
    const [answer, setAnswer] = useState('');
    const [tip, setTip] = useState('');

    const load = () => {
        invoke('get_corrections')
            .then((map) => {
                setItems(Object.entries(map ?? {}).sort(([a], [b]) => a.localeCompare(b)));
            })
            .catch(() => {});
    };

    useEffect(load, []);

    const flash = (msg) => {
        setTip(msg);
        setTimeout(() => setTip(''), 2500);
    };

    const add = () => {
        const c = cipher.trim().toUpperCase();
        const a = answer.trim().toUpperCase();
        if (!c || !a) {
            flash('密文和答案都不能为空');
            return;
        }
        invoke('save_correction', { cipher: c, answer: a })
            .then(() => {
                setCipher('');
                setAnswer('');
                flash('已保存');
                load();
            })
            .catch(() => flash('保存失败'));
    };

    const del = (c) => {
        invoke('del_correction', { cipher: c })
            .then(() => {
                flash('已删除');
                load();
            })
            .catch(() => flash('删除失败'));
    };

    return (
        <div className='max-w-[640px]'>
            <Card shadow='none' className='border-1 border-default-200 p-[12px] mb-[12px]'>
                <div className='font-bold mb-[8px]'>添加 / 更新词条</div>
                <div className='flex gap-[8px] items-center'>
                    <Input
                        size='sm'
                        label='密文'
                        placeholder='如 QHJDWLYH'
                        value={cipher}
                        onValueChange={(v) => setCipher(v.toUpperCase())}
                        className='flex-1'
                    />
                    <span className='text-default-400'>→</span>
                    <Input
                        size='sm'
                        label='答案'
                        placeholder='如 NEGATIVE'
                        value={answer}
                        onValueChange={(v) => setAnswer(v.toUpperCase())}
                        className='flex-1'
                    />
                    <Button size='sm' color='primary' onPress={add}>
                        保存
                    </Button>
                </div>
                {tip && <div className='text-tiny text-primary mt-[6px]'>{tip}</div>}
                <div className='text-tiny text-default-400 mt-[8px]'>
                    词库中的记录会在识别时优先参考，命中后答案旁显示 ★。词库只增不改时无需清理，错误记录可直接删除。
                </div>
            </Card>
            <Card shadow='none' className='border-1 border-default-200 p-[12px]'>
                <div className='font-bold mb-[8px]'>词库记录（{items.length} 条）</div>
                {items.length === 0 && (
                    <div className='text-default-400 text-tiny'>暂无记录。识别结果词表未命中时（? 标记），可在设置页或未来版本中补充。</div>
                )}
                {items.map(([c, a]) => (
                    <div
                        key={c}
                        className='flex items-center gap-[8px] py-[4px] border-b-1 border-default-100 last:border-b-0'
                    >
                        <span className='font-mono font-bold w-[140px] truncate'>{c}</span>
                        <span className='text-default-400'>→</span>
                        <span className='font-mono text-primary flex-1 truncate'>{a}</span>
                        <Button size='sm' variant='light' color='danger' onPress={() => del(c)}>
                            删除
                        </Button>
                    </div>
                ))}
            </Card>
        </div>
    );
}

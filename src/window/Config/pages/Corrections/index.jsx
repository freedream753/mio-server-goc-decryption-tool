// 自学习词库设置页：维护密文 -> 答案映射（answers.json）
// 识别管线在词表未命中时优先参考该词库（命中显示 ★）
// 核心答案词库（45 词）为词表反查兜底的唯一数据源，改动需二次确认
import { invoke } from '@tauri-apps/api';
import {
    Button, Card, Chip, Input, Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
    Textarea, useDisclosure,
} from '@nextui-org/react';
import React, { useEffect, useState } from 'react';

export default function Corrections() {
    const [items, setItems] = useState([]);
    const [cipher, setCipher] = useState('');
    const [answer, setAnswer] = useState('');
    const [tip, setTip] = useState('');
    // 核心答案词库
    const [words, setWords] = useState([]);
    const [isCustom, setIsCustom] = useState(false); // 当前是否为自定义词表
    const [editingWords, setEditingWords] = useState(false);
    const [draft, setDraft] = useState('');
    const [confirmMode, setConfirmMode] = useState('save'); // save | reset
    const { isOpen, onOpen, onOpenChange } = useDisclosure();

    const load = () => {
        invoke('get_corrections')
            .then((map) => {
                setItems(Object.entries(map ?? {}).sort(([a], [b]) => a.localeCompare(b)));
            })
            .catch(() => {});
        invoke('get_answer_words')
            .then((w) => setWords(w ?? []))
            .catch(() => {});
        // 自定义判定由后端下发（words.json 是否存在），避免按词数与内置默认硬耦合
        invoke('is_custom_words')
            .then(setIsCustom)
            .catch(() => {});
    };

    useEffect(load, []);

    const flash = (msg, danger = false) => {
        setTip(msg);
        setTimeout(() => setTip(''), 3000);
        if (danger) console.warn(msg);
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

    // ===== 核心答案词库编辑 =====
    const startEdit = () => {
        setDraft(words.join('\n'));
        setEditingWords(true);
    };

    const requestSave = () => {
        const list = draft.split(/[^A-Za-z]+/).filter(Boolean);
        if (list.length === 0) {
            flash('词表不能为空');
            return;
        }
        setConfirmMode('save');
        onOpen();
    };

    const doSave = () => {
        const list = draft.split(/[^A-Za-z]+/).filter(Boolean).map((w) => w.toUpperCase());
        invoke('save_answer_words', { words: list })
            .then((n) => {
                flash(`词表已保存（${n} 词），立即生效`);
                setEditingWords(false);
                load();
            })
            .catch((e) => flash(`保存失败：${e}`, true));
    };

    const doReset = () => {
        invoke('reset_answer_words')
            .then(() => {
                flash('已恢复内置默认词表，立即生效');
                setEditingWords(false);
                load();
            })
            .catch((e) => flash(`恢复失败：${e}`, true));
    };

    return (
        <div className='max-w-[640px]'>
            {/* 核心答案词库：词表反查兜底的唯一数据源，改动需二次确认 */}
            <Card shadow='none' className='border-1 border-default-200 p-[12px] mb-[12px]'>
                <div className='flex items-center justify-between mb-[6px]'>
                    <div className='font-bold'>核心答案词库（{words.length} 词）</div>
                    {isCustom && <Chip size='sm' color='warning' variant='flat'>自定义中</Chip>}
                </div>
                <div className='text-tiny text-default-500 mb-[8px]'>
                    这 {words.length} 个单词是游戏答案全集（提取自官方前/后移对应表）。程序由它们自动派生
                    「密文 → 方向 + 位数 + 答案」的全部映射，是识别失败时<b>词表反查兜底（≡ 标记）</b>与
                    <b>手动位移校验</b>的唯一数据源——表格行本身是冗余的（答案词按移位公式即可算出密文），
                    因此只需维护词表本身。
                </div>
                {!editingWords ? (
                    <>
                        <div className='flex flex-wrap gap-[4px] mb-[8px]'>
                            {words.map((w) => (
                                <Chip key={w} size='sm' variant='flat' className='font-mono'>{w}</Chip>
                            ))}
                        </div>
                        <div className='flex gap-[8px]'>
                            <Button size='sm' color='primary' variant='flat' onPress={startEdit}>编辑词表</Button>
                            <Button size='sm' color='danger' variant='light' isDisabled={!isCustom} onPress={() => { setConfirmMode('reset'); onOpen(); }}>
                                恢复默认
                            </Button>
                        </div>
                    </>
                ) : (
                    <>
                        <Textarea
                            size='sm'
                            value={draft}
                            onValueChange={setDraft}
                            minRows={10}
                            maxRows={20}
                            placeholder='每行一个单词，A-Z，3~12 位'
                            description={`当前 ${draft.split(/[^A-Za-z]+/).filter(Boolean).length} 词（自动去重、大写）`}
                            className='mb-[8px]'
                        />
                        <div className='flex gap-[8px]'>
                            <Button size='sm' color='danger' variant='flat' onPress={requestSave}>保存词表</Button>
                            <Button size='sm' variant='light' onPress={() => setEditingWords(false)}>取消</Button>
                        </div>
                    </>
                )}
                {tip && <div className='text-tiny text-primary mt-[6px]'>{tip}</div>}
            </Card>

            {/* 二次确认弹窗：说明改动严重性 */}
            <Modal isOpen={isOpen} onOpenChange={onOpenChange} size='lg'>
                <ModalContent>
                    {(onClose) => (
                        <>
                            <ModalHeader className='text-danger'>
                                {confirmMode === 'save' ? '确认修改核心答案词库？' : '确认恢复默认词表？'}
                            </ModalHeader>
                            <ModalBody>
                                {confirmMode === 'save' ? (
                                    <div className='text-small space-y-[6px]'>
                                        <div className='font-bold'>修改将立即生效并永久保存，请确认了解以下风险：</div>
                                        <div>1. <b>删除或写错单词</b>：对应谜题的词表反查会失败，自动降级为手动 ± 位移，识别成功率明显下降；</div>
                                        <div>2. <b>添加非答案词</b>：可能与现有密文产生映射冲突，把原本正确的答案顶成错误答案，且结果仍显示「已验证 ✓」，极具误导性；</div>
                                        <div>3. 词表是<b>分级输出兜底（词表反查）的唯一数据源</b>，仅在官方更新词库时才应修改。</div>
                                    </div>
                                ) : (
                                    <div className='text-small space-y-[6px]'>
                                        <div>将删除自定义词表文件（words.json）并恢复内置默认词表，<b>自定义修改无法找回</b>。</div>
                                        <div className='text-default-500'>内置词表提取自官方前/后移对应表，通常无需改动。</div>
                                    </div>
                                )}
                            </ModalBody>
                            <ModalFooter>
                                <Button size='sm' variant='light' onPress={onClose}>取消</Button>
                                <Button size='sm' color='danger' onPress={confirmMode === 'save' ? doSave : doReset}>
                                    {confirmMode === 'save' ? '确认保存' : '确认恢复'}
                                </Button>
                            </ModalFooter>
                        </>
                    )}
                </ModalContent>
            </Modal>

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

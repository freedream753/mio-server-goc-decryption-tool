import { useNavigate, useLocation } from 'react-router-dom';
import { AiFillAppstore, AiFillInfoCircle } from 'react-icons/ai';
import { useTranslation } from 'react-i18next';
import { MdKeyboardAlt } from 'react-icons/md';
import { Button } from '@nextui-org/react';
import React from 'react';

export default function SideBar() {
    const { t } = useTranslation();
    const navigate = useNavigate();
    const location = useLocation();

    function setStyle(pathname) {
        return location.pathname.includes(pathname) ? 'flat' : 'light';
    }

    return (
        <div className='mx-[12px] overflow-y-auto'>
            <Button
                fullWidth
                size='lg'
                variant={setStyle('/general')}
                className='mb-[5px]'
                onPress={() => {
                    navigate('/general');
                }}
                startContent={<AiFillAppstore className='text-[24px]' />}
            >
                <div className='w-full'>{t('config.general.label')}</div>
            </Button>
            <Button
                fullWidth
                size='lg'
                variant={setStyle('/hotkey')}
                className='mb-[5px]'
                onPress={() => {
                    navigate('/hotkey');
                }}
                startContent={<MdKeyboardAlt className='text-[24px]' />}
            >
                <div className='w-full'>{t('config.hotkey.label')}</div>
            </Button>
            <Button
                fullWidth
                size='lg'
                variant={setStyle('/about')}
                className='mb-[5px]'
                onPress={() => {
                    navigate('/about');
                }}
                startContent={<AiFillInfoCircle className='text-[24px]' />}
            >
                <div className='w-full'>{t('config.about.label')}</div>
            </Button>
        </div>
    );
}

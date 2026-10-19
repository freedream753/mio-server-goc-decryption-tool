import React from 'react';
import { Card, CardBody, Link } from '@nextui-org/react';
import { useTranslation } from 'react-i18next';

// 原项目仓库（GPL-3.0）
const UPSTREAM_REPO = 'https://github.com/pot-app/pot-desktop';
// 本修改版仓库
const FORK_REPO = 'https://github.com/freedream753/mio-server-goc-decryption-tool';

export default function About() {
    const { t } = useTranslation();

    return (
        <>
            <Card className='mb-[10px]'>
                <CardBody>
                    <div className='flex items-center gap-[15px]'>
                        <img
                            alt='GOC logo'
                            src='goc.png'
                            className='h-[64px] w-[64px]'
                            draggable={false}
                        />
                        <div>
                            <h2 className='text-xl font-bold'>{t('config.about.name')}</h2>
                            <p className='text-sm text-default-500'>{t('config.about.desc')}</p>
                        </div>
                    </div>
                </CardBody>
            </Card>
            <Card className='mb-[10px]'>
                <CardBody>
                    <h3 className='mb-[5px] font-bold'>{t('config.about.license_title')}</h3>
                    <p className='whitespace-pre-line text-sm text-default-600'>
                        {t('config.about.license_desc')}
                    </p>
                </CardBody>
            </Card>
            <Card>
                <CardBody className='gap-[8px]'>
                    <h3 className='font-bold'>{t('config.about.links_title')}</h3>
                    <div>
                        <Link
                            href={UPSTREAM_REPO}
                            isExternal
                            showAnchorIcon
                            size='sm'
                        >
                            pot-app/pot-desktop（原项目 · GPL-3.0）
                        </Link>
                    </div>
                    <div>
                        <Link
                            href={FORK_REPO}
                            isExternal
                            showAnchorIcon
                            size='sm'
                        >
                            {t('config.about.links_fork')}
                        </Link>
                    </div>
                </CardBody>
            </Card>
        </>
    );
}

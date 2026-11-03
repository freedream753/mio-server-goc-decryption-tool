import { Navigate } from 'react-router-dom';

import About from '../pages/About';
import Corrections from '../pages/Corrections';
import General from '../pages/General';
import Hotkey from '../pages/Hotkey';

const routes = [
    {
        path: '/general',
        element: <General />,
    },
    {
        path: '/hotkey',
        element: <Hotkey />,
    },
    {
        path: '/corrections',
        element: <Corrections />,
    },
    {
        path: '/about',
        element: <About />,
    },
    {
        path: '/',
        element: <Navigate to='/general' />,
    },
];

export default routes;

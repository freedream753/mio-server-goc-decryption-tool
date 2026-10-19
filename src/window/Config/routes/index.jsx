import { Navigate } from 'react-router-dom';

import About from '../pages/About';
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
        path: '/about',
        element: <About />,
    },
    {
        path: '/',
        element: <Navigate to='/general' />,
    },
];

export default routes;

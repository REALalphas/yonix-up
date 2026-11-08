import React from 'react';
import { createRoot } from 'react-dom/client';
import { installControlMotion } from '@alpha/core';
import '@alpha/core/fonts.css';
import '@alpha/core/styles.css';
import './app.css';
import { App } from './App';
const cleanup = installControlMotion();
if (import.meta.hot)
    import.meta.hot.dispose(cleanup);
createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);

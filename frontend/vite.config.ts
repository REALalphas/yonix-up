import { defineConfig } from 'vite';
export default defineConfig({
    root: 'frontend',
    base: '/',
    build: { outDir: '../web', emptyOutDir: true },
    server: {
        proxy: {
            '/api': 'http://127.0.0.1:9847'
        }
    }
});

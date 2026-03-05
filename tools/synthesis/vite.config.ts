import { sveltekit } from '@sveltejs/kit/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import tailwindcss from '@tailwindcss/vite';
import basicSsl from '@vitejs/plugin-basic-ssl';
import { defineConfig } from 'vite';

export default defineConfig({
	plugins: [tailwindcss(), basicSsl(), sveltekit(), svelteTesting()],
	server: {
		host: '0.0.0.0',
		port: 5174,
		strictPort: true
	},
	test: {
		include: ['src/**/*.test.ts'],
		environment: 'jsdom',
		alias: {
			$lib: new URL('./src/lib', import.meta.url).pathname
		},
		setupFiles: ['./src/test-setup.ts']
	}
});

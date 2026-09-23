import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	// Resolve package symlinks to their real workspace/store locations so
	// isolated Bun installs retain each package's dependency graph. Dedupe
	// Svelte explicitly for the source-linked shared UI.
	resolve: { dedupe: ['svelte'] },
	plugins: [tailwindcss(), sveltekit()],
	server: {
		port: 5175,
		strictPort: true,
		proxy: {
			'/api': {
				target: process.env.HOME_SERVER_URL ?? 'http://127.0.0.1:25600',
				changeOrigin: true,
				ws: true
			}
		}
	}
});

import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// Vite 7 keeps esbuild's nearest-tsconfig behavior; Vite 8's Rolldown transform
// walks into the monorepo root tsconfig and its unavailable Expo reference.
export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	server: {
		port: 5174,
		strictPort: true,
		proxy: {
			'/api': { target: 'http://127.0.0.1:25600', changeOrigin: true, ws: true }
		}
	}
});

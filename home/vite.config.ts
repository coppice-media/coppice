import tailwindcss from '@tailwindcss/vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
	// The shared `@stump/ui` package is a `file:` dependency; resolving from
	// the link position keeps one copy of the graphql stack per app.
	resolve: { preserveSymlinks: true },
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

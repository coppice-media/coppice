import { createRootRoute, HeadContent, Outlet, Scripts } from '@tanstack/react-router'
import { RootProvider } from 'fumadocs-ui/provider/tanstack'

import appCss from '@/styles/app.css?url'

export const Route = createRootRoute({
	head: () => ({
		meta: [
			{
				charSet: 'utf-8',
			},
			{
				name: 'viewport',
				content: 'width=device-width, initial-scale=1',
			},
			{
				title: 'Coppice Docs',
			},
			{
				name: 'robots',
				content: 'index,follow',
			},
			{
				name: 'description',
				content:
					'Free, open source, self-hosting for your comic books, manga and digital book collections.',
			},
			{
				property: 'og:title',
				content: 'Coppice',
			},
			{
				property: 'og:description',
				content:
					'Free, open source, self-hosting for your comic books, manga and digital book collections.',
			},
			{
				property: 'og:type',
				content: 'website',
			},
			{
				property: 'og:locale',
				content: 'en_US',
			},
			{
				property: 'og:site_name',
				content: 'Coppice',
			},
		],
		links: [
			{ rel: 'stylesheet', href: appCss },
			{ rel: 'icon', href: '/favicon.ico' },
			{
				rel: 'icon',
				type: 'image/png',
				sizes: '32x32',
				href: '/favicon.png',
			},
		],
	}),
	component: RootComponent,
})

function RootComponent() {
	return (
		<html suppressHydrationWarning>
			<head>
				<HeadContent />
			</head>
			<body className="flex min-h-screen flex-col">
				<RootProvider>
					<Outlet />
				</RootProvider>
				<Scripts />
			</body>
		</html>
	)
}

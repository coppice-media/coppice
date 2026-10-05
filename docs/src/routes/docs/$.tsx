import { createFileRoute, notFound } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import browserCollections from 'collections/browser'
import { useFumadocsLoader } from 'fumadocs-core/source/client'
import { DocsLayout } from 'fumadocs-ui/layouts/docs'
import {
	DocsBody,
	DocsDescription,
	DocsPage,
	DocsTitle,
	EditOnGitHub,
} from 'fumadocs-ui/layouts/docs/page'
import { Suspense } from 'react'

import { useMDXComponents } from '@/components/mdx'
import { baseOptions } from '@/lib/layout.shared'
import { gitConfig } from '@/lib/shared'
import { slugsToMarkdownPath, source } from '@/lib/source'

export const Route = createFileRoute('/docs/$')({
	component: Page,
	loader: async ({ params }) => {
		const slugs = params._splat?.split('/') ?? []
		const data = await serverLoader({ data: slugs })
		await clientLoader.preload(data.path)
		return data
	},
})

const serverLoader = createServerFn({
	method: 'GET',
})
	.inputValidator((slugs: string[]) => slugs)
	.handler(async ({ data: slugs }) => {
		const page = source.getPage(slugs)
		if (!page) throw notFound()

		return {
			path: page.path,
			markdownUrl: slugsToMarkdownPath(page.slugs).url,
			pageTree: await source.serializePageTree(source.getPageTree()),
			lastModified: page.data.lastModified,
		}
	})

const clientLoader = browserCollections.docs.createClientLoader({
	component(
		{ toc, frontmatter, default: MDX },
		{
			// markdownUrl,
			path,
			lastModified,
		}: {
			markdownUrl: string
			path: string
			lastModified?: Date | null
		},
	) {
		return (
			<>
				<div className="pointer-events-none absolute inset-0 -z-10 h-full w-full overflow-x-clip">
					<div className="pointer-events-none absolute top-0 right-0 -z-10 h-256 w-5xl translate-x-1/2 -translate-y-1/2 rounded-full bg-amber-500/10 [mask-image:var(--mask)] [--mask:radial-gradient(circle_at_center,red,transparent_69%)] [webkit-mask-image:var(--mask)] max-md:hidden xl:right-1/2" />
					<div className="pointer-events-none fixed top-0 right-0 -z-10 h-256 w-5xl translate-x-1/2 -translate-y-1/2 rounded-full bg-amber-500/5 [mask-image:var(--mask)] [--mask:radial-gradient(circle_at_center,red,transparent_69%)] [webkit-mask-image:var(--mask)] max-md:hidden xl:right-1/2" />
					<div className="bg-dot-matrix-xl pointer-events-none absolute top-0 right-0 -z-10 h-256 w-5xl translate-x-1/2 -translate-y-1/2 [mask-image:var(--mask)] [--mask:radial-gradient(circle_at_center_top,red,transparent)] [webkit-mask-image:var(--mask)] max-md:hidden xl:right-1/2 dark:opacity-80" />
				</div>

				<DocsPage
					toc={toc}
					tableOfContent={{
						style: 'clerk',
					}}
				>
					{lastModified && (
						<p className="-mb-4 text-sm text-fd-muted-foreground">
							Last updated on{' '}
							{Intl.DateTimeFormat('en-US', { dateStyle: 'long' }).format(new Date(lastModified))}
						</p>
					)}
					<div className="mb-4 flex items-center justify-between">
						<div className="flex-1">
							<DocsTitle>{frontmatter.title}</DocsTitle>
							<DocsDescription>{frontmatter.description}</DocsDescription>
						</div>
						<div className="flex items-center gap-2">
							<EditOnGitHub
								href={`https://github.com/${gitConfig.user}/${gitConfig.repo}/blob/${gitConfig.branch}/docs/content/docs/${path}`}
							/>
						</div>
					</div>
					<DocsBody>
						<MDX components={useMDXComponents()} />
					</DocsBody>
				</DocsPage>
			</>
		)
	},
})

function Page() {
	const { path, pageTree, markdownUrl, lastModified } = useFumadocsLoader(Route.useLoaderData())

	return (
		<DocsLayout {...baseOptions()} tree={pageTree}>
			<Suspense>{clientLoader.useContent(path, { markdownUrl, path, lastModified })}</Suspense>
		</DocsLayout>
	)
}

<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { ConsoleAuthorsDocument } from '$lib/graphql/generated/graphql';
	import { countNoun } from '$lib/format';
	import { offsetPage } from '$lib/library';
	import EntityGrid from '$lib/components/library/EntityGrid.svelte';
	import EntityNav from '$lib/components/library/EntityNav.svelte';
	import Pager from '$lib/components/library/Pager.svelte';
	import { entityHref, useEntityScope } from '$lib/components/library/scope.svelte';
	import type { EntityTileData } from '$lib/components/library/entities';

	const PAGE_SIZE = 24;

	const scope = useEntityScope();

	const authorsQuery = createQuery(() => ({
		queryKey: ['authors', scope.libraryId, scope.search, scope.page],
		queryFn: () =>
			request(ConsoleAuthorsDocument, {
				search: scope.search,
				libraryId: scope.libraryId,
				pagination: { offset: { page: scope.page, pageSize: PAGE_SIZE } }
			}),
		enabled: browser
	}));
	const authors = $derived(authorsQuery.data?.authors.nodes ?? []);
	const pageInfo = $derived(offsetPage(authorsQuery.data?.authors.pageInfo));

	const entities = $derived<EntityTileData[]>(
		authors.map((author) => ({
			key: author.name,
			name: author.name,
			// The `authors` query already carries the books behind each name, so
			// the tile needs no count query of its own.
			bookCount: author.books.length,
			detail: author.series.length
				? countNoun(author.series.length, 'series', 'series')
				: undefined,
			href: entityHref(scope, { author: author.name })
		}))
	);
</script>

<svelte:head>
	<title>Authors · Coppice</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<EntityNav
		{scope}
		title="Authors"
		description="Everyone credited as a writer, from the metadata of your books."
		searchPlaceholder="Brian K. Vaughan"
	/>

	{#if authorsQuery.isPending}
		<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
			{#each { length: 9 } as _, index (index)}
				<Skeleton class="h-16 rounded-xl" />
			{/each}
		</div>
	{:else if authorsQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load authors</AlertTitle>
			<AlertDescription>
				{authorsQuery.error instanceof Error ? authorsQuery.error.message : 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else}
		<EntityGrid
			{entities}
			emptyTitle="No authors yet"
			emptyDescription="Authors come from book metadata; scan or fetch metadata for a library first."
		/>
		<Pager page={pageInfo} noun="author" onpage={(next) => scope.setPage(next)} />
	{/if}
</div>

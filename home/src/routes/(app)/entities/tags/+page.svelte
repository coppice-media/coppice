<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { ConsoleTagsDocument } from '$lib/graphql/generated/graphql';
	import EntityGrid from '$lib/components/library/EntityGrid.svelte';
	import EntityNav from '$lib/components/library/EntityNav.svelte';
	import Pager from '$lib/components/library/Pager.svelte';
	import { entityHref, slicePage, useEntityScope } from '$lib/components/library/scope.svelte';
	import type { EntityTileData } from '$lib/components/library/entities';

	const PAGE_SIZE = 24;

	const scope = useEntityScope();

	const tagsQuery = createQuery(() => ({
		queryKey: ['tags'],
		queryFn: () => request(ConsoleTagsDocument, {}),
		enabled: browser
	}));

	const tags = $derived(tagsQuery.data?.tags ?? []);
	const matching = $derived(
		scope.search
			? tags.filter((tag) => tag.name.toLowerCase().includes(scope.search!.toLowerCase()))
			: tags
	);
	const paged = $derived(slicePage(matching, scope.page, PAGE_SIZE));

	const entities = $derived<EntityTileData[]>(
		paged.items.map((tag) => ({
			key: `tag:${scope.libraryId}:${tag.id}`,
			name: tag.name,
			// Komga round trips keep genres and tags apart; the kind says which.
			detail: tag.kind.toLowerCase() === 'tag' ? undefined : tag.kind.toLowerCase(),
			filter: {
				series: { libraryId: { eq: scope.libraryId ?? '' } },
				tags: { eq: tag.name }
			},
			href: entityHref(scope, { tag: tag.name })
		}))
	);
</script>

<svelte:head>
	<title>Tags · Stump</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<EntityNav
		{scope}
		title="Tags"
		description="Every tag on this server; the counts are for the selected library."
		searchPlaceholder="sci-fi"
	/>

	{#if tagsQuery.isPending}
		<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
			{#each { length: 9 } as _, index (index)}
				<Skeleton class="h-16 rounded-xl" />
			{/each}
		</div>
	{:else if tagsQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load tags</AlertTitle>
			<AlertDescription>
				{tagsQuery.error instanceof Error ? tagsQuery.error.message : 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else}
		<EntityGrid
			{entities}
			emptyTitle="No tags yet"
			emptyDescription="Tag a book or series and it shows up here with its book count."
		/>
		<Pager
			page={entities.length ? paged.pageInfo : null}
			noun="tag"
			onpage={(next) => scope.setPage(next)}
		/>
	{/if}
</div>

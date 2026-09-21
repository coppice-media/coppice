<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { ConsoleLibraryPublishersDocument } from '$lib/graphql/generated/graphql';
	import EntityGrid from '$lib/components/library/EntityGrid.svelte';
	import EntityNav from '$lib/components/library/EntityNav.svelte';
	import Pager from '$lib/components/library/Pager.svelte';
	import { entityHref, slicePage, useEntityScope } from '$lib/components/library/scope.svelte';
	import type { EntityTileData } from '$lib/components/library/entities';

	const PAGE_SIZE = 24;

	const scope = useEntityScope();

	const publishersQuery = createQuery(() => ({
		queryKey: ['libraryPublishers', scope.libraryId],
		queryFn: () =>
			request(ConsoleLibraryPublishersDocument, { id: scope.libraryId ?? '' }),
		enabled: browser && !!scope.libraryId
	}));

	const publishers = $derived(publishersQuery.data?.libraryById?.publishers ?? []);
	const matching = $derived(
		scope.search
			? publishers.filter((name) => name.toLowerCase().includes(scope.search!.toLowerCase()))
			: publishers
	);
	const paged = $derived(slicePage(matching, scope.page, PAGE_SIZE));

	const entities = $derived<EntityTileData[]>(
		paged.items.map((name) => ({
			key: `publisher:${scope.libraryId}:${name}`,
			name,
			// No aggregate exists for publishers, so each visible tile counts
			// its own books within the selected library.
			filter: {
				series: { libraryId: { eq: scope.libraryId ?? '' } },
				metadata: { publisher: { eq: name } }
			},
			href: entityHref(scope, { publisher: name })
		}))
	);
</script>

<svelte:head>
	<title>Publishers · Coppice</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<EntityNav
		{scope}
		title="Publishers"
		description="Publishers credited by the books of the selected library."
		searchPlaceholder="Image Comics"
	/>

	{#if scope.isPending || publishersQuery.isPending}
		<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
			{#each { length: 9 } as _, index (index)}
				<Skeleton class="h-16 rounded-xl" />
			{/each}
		</div>
	{:else if publishersQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load publishers</AlertTitle>
			<AlertDescription>
				{publishersQuery.error instanceof Error
					? publishersQuery.error.message
					: 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else}
		<EntityGrid
			{entities}
			emptyTitle="No publishers yet"
			emptyDescription="Publishers come from book metadata; scan or fetch metadata for this library first."
		/>
		<Pager
			page={entities.length ? paged.pageInfo : null}
			noun="publisher"
			onpage={(next) => scope.setPage(next)}
		/>
	{/if}
</div>

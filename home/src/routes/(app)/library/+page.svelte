<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createQuery } from '@tanstack/svelte-query';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import { ConsoleLibrariesDocument } from '$lib/graphql/generated/graphql';
	import { offsetPage } from '$lib/library';
	import CreateLibraryDialog from '$lib/components/library/CreateLibraryDialog.svelte';
	import LibraryCard from '$lib/components/library/LibraryCard.svelte';
	import Pager from '$lib/components/library/Pager.svelte';

	const PAGE_SIZE = 12;

	let page = $state(1);
	let createOpen = $state(false);

	const librariesQuery = createQuery(() => ({
		queryKey: ['libraries', page],
		queryFn: () =>
			request(ConsoleLibrariesDocument, {
				pagination: { offset: { page, pageSize: PAGE_SIZE } }
			}),
		enabled: browser
	}));
	const libraries = $derived(librariesQuery.data?.libraries.nodes ?? []);
	const pageInfo = $derived(offsetPage(librariesQuery.data?.libraries.pageInfo));
</script>

<svelte:head>
	<title>Library · Coppice</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-center gap-3">
		<div class="mr-auto">
			<h1 class="text-2xl font-semibold tracking-tight">Library</h1>
			<p class="text-sm text-muted-foreground">
				Every library on this server, with what the last scan found.
			</p>
		</div>
		<Button variant="outline" href={resolve('/entities/authors')}>Browse by author</Button>
		<Button onclick={() => (createOpen = true)}>
			<PlusIcon data-icon="inline-start" />
			New library
		</Button>
	</div>

	{#if librariesQuery.isPending}
		<div class="grid gap-4 md:grid-cols-2">
			{#each { length: 4 } as _, index (index)}
				<Skeleton class="h-56 rounded-xl" />
			{/each}
		</div>
	{:else if librariesQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load libraries</AlertTitle>
			<AlertDescription>
				{librariesQuery.error instanceof Error ? librariesQuery.error.message : 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else if libraries.length === 0}
		<Empty class="rounded-xl border border-dashed bg-card">
			<EmptyHeader>
				<EmptyTitle>No libraries yet</EmptyTitle>
				<EmptyDescription>
					Point a library at a folder inside one of this server's library roots and Coppice will
					scan it into series and books.
				</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else}
		<div class="grid gap-4 md:grid-cols-2">
			{#each libraries as library (library.id)}
				<LibraryCard {library} />
			{/each}
		</div>
		<Pager page={pageInfo} noun="library" plural="libraries" onpage={(next) => (page = next)} />
	{/if}
</div>

<CreateLibraryDialog bind:open={createOpen} />

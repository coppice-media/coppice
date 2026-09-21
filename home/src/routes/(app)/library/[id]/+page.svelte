<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createQuery } from '@tanstack/svelte-query';
	import LayoutGridIcon from '@lucide/svelte/icons/layout-grid';
	import TableIcon from '@lucide/svelte/icons/table';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleBooksDocument,
		ConsoleLibraryDetailDocument,
		ConsoleLibrarySeriesDocument,
		type ReadingStatus
	} from '$lib/graphql/generated/graphql';
	import { bytesLabel, countLabel, relativeTime } from '$lib/format';
	import {
		LIBRARY_PATTERN_LABELS,
		LIBRARY_TYPE_LABELS,
		NO_FACETS,
		bookFilter,
		offsetPage,
		type BookFacets
	} from '$lib/library';
	import BookFilters from '$lib/components/library/BookFilters.svelte';
	import BookTable from '$lib/components/library/BookTable.svelte';
	import Pager from '$lib/components/library/Pager.svelte';
	import SeriesGrid from '$lib/components/library/SeriesGrid.svelte';
	import SeriesTable from '$lib/components/library/SeriesTable.svelte';
	import CrossPointSendButton from '$lib/components/library/CrossPointSendButton.svelte';

	const SERIES_PAGE_SIZE = 24;
	const BOOKS_PAGE_SIZE = 25;

	const libraryId = $derived(page.params.id ?? '');
	let selected = $state<string[]>([]);

	// The screen's state lives in the URL: the entity grids link straight to a
	// filtered view, and back/forward keeps working.
	const params = $derived(page.url.searchParams);
	const tab = $derived(params.get('tab') === 'books' ? 'books' : 'series');
	const view = $derived(params.get('view') === 'table' ? 'table' : 'grid');
	const pageNumber = $derived(Math.max(1, Number(params.get('page') ?? '1') || 1));
	const facets = $derived<BookFacets>({
		status: (params.get('status') as ReadingStatus | null) ?? null,
		format: params.get('format'),
		tag: params.get('tag'),
		publisher: params.get('publisher'),
		author: params.get('author'),
		search: params.get('search')
	});

	function navigate(patch: Record<string, string | null>, resetPage = true): void {
		selected = [];
		const url = new URL(page.url);
		for (const [key, value] of Object.entries(patch)) {
			if (value === null || value === '') url.searchParams.delete(key);
			else url.searchParams.set(key, value);
		}
		if (resetPage) url.searchParams.delete('page');
		void goto(url, { replaceState: true, noScroll: true, keepFocus: true });
	}

	const libraryQuery = createQuery(() => ({
		queryKey: ['library', libraryId],
		queryFn: () => request(ConsoleLibraryDetailDocument, { id: libraryId }),
		enabled: browser && !!libraryId
	}));
	const library = $derived(libraryQuery.data?.libraryById ?? null);

	const seriesQuery = createQuery(() => ({
		queryKey: ['series', 'library', libraryId, facets.search, facets.status, pageNumber],
		queryFn: () =>
			request(ConsoleLibrarySeriesDocument, {
				filter: {
					libraryId: { eq: libraryId },
					...(facets.status ? { readingStatus: { is: facets.status } } : {}),
					...(facets.search ? { name: { contains: facets.search } } : {})
				},
				orderBy: [{ series: { field: 'NAME', direction: 'ASC' } }],
				pagination: { offset: { page: pageNumber, pageSize: SERIES_PAGE_SIZE } }
			}),
		enabled: browser && !!libraryId && tab === 'series'
	}));
	const series = $derived(seriesQuery.data?.series.nodes ?? []);
	const seriesPage = $derived(offsetPage(seriesQuery.data?.series.pageInfo));

	const booksQuery = createQuery(() => ({
		queryKey: ['books', 'library', libraryId, facets, pageNumber],
		queryFn: () =>
			request(ConsoleBooksDocument, {
				filter: bookFilter({ libraryId }, facets),
				orderBy: [{ media: { field: 'NAME', direction: 'ASC' } }],
				pagination: { offset: { page: pageNumber, pageSize: BOOKS_PAGE_SIZE } }
			}),
		enabled: browser && !!libraryId && tab === 'books'
	}));
	const books = $derived(booksQuery.data?.media.nodes ?? []);
	const booksPage = $derived(offsetPage(booksQuery.data?.media.pageInfo));

	// Book-level facets have no counterpart on the series filter, so setting one
	// is only meaningful on the books tab; sending the user there beats
	// silently ignoring the filter.
	function applyFacets(patch: Partial<BookFacets>): void {
		const bookOnly = ['format', 'tag', 'publisher', 'author'] as const;
		const switchesTab = bookOnly.some((key) => patch[key]);
		navigate({
			...Object.fromEntries(Object.entries(patch).map(([key, value]) => [key, value ?? null])),
			...(switchesTab ? { tab: 'books' } : {})
		});
	}
</script>

<svelte:head>
	<title>{library ? `${library.name} · Coppice` : 'Library · Coppice'}</title>
</svelte:head>

<div class="flex flex-col gap-6">
	{#if libraryQuery.isPending}
		<Skeleton class="h-28 rounded-xl" />
	{:else if libraryQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load this library</AlertTitle>
			<AlertDescription>
				{libraryQuery.error instanceof Error ? libraryQuery.error.message : 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else if !library}
		<Alert variant="destructive">
			<AlertTitle>Library not found</AlertTitle>
			<AlertDescription>
				It may have been deleted, or it is not shared with your account.
			</AlertDescription>
		</Alert>
	{:else}
		<div class="flex flex-wrap items-start gap-3">
			<div class="mr-auto">
				<div class="flex items-center gap-2">
					<h1 class="text-2xl font-semibold tracking-tight">
						{library.emoji ? `${library.emoji} ` : ''}{library.name}
					</h1>
					{#if library.status !== 'READY'}
						<Badge variant="destructive">{library.status}</Badge>
					{/if}
				</div>
				<p class="text-sm text-muted-foreground">
					{LIBRARY_TYPE_LABELS[library.config.libraryType]} ·
					{LIBRARY_PATTERN_LABELS[library.config.libraryPattern]} · last scanned
					{relativeTime(library.lastScannedAt)}
				</p>
			</div>
			<Button variant="outline" href={resolve('/library')}>All libraries</Button>
		</div>

		<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
			{#each [['Series', countLabel(library.stats.seriesCount)], ['Books', countLabel(library.stats.bookCount)], ['Finished', countLabel(library.stats.completedBooks)], ['On disk', bytesLabel(library.stats.totalBytes)]] as [label, value] (label)}
				<div class="rounded-xl border bg-card px-4 py-3">
					<div class="text-sm text-muted-foreground">{label}</div>
					<div class="text-2xl font-medium tabular-nums">{value}</div>
				</div>
			{/each}
		</div>

		<div class="flex flex-wrap items-center gap-3">
			<Tabs.Root value={tab} onValueChange={(value) => navigate({ tab: value })}>
				<Tabs.List>
					<Tabs.Trigger value="series">Series</Tabs.Trigger>
					<Tabs.Trigger value="books">Books</Tabs.Trigger>
				</Tabs.List>
			</Tabs.Root>
			<div class="ml-auto flex items-center gap-1">
				<Button
					size="icon-sm"
					variant={view === 'grid' ? 'secondary' : 'ghost'}
					aria-label="Grid view"
					aria-pressed={view === 'grid'}
					onclick={() => navigate({ view: 'grid' }, false)}
				>
					<LayoutGridIcon />
				</Button>
				<Button
					size="icon-sm"
					variant={view === 'table' ? 'secondary' : 'ghost'}
					aria-label="Table view"
					aria-pressed={view === 'table'}
					onclick={() => navigate({ view: 'table' }, false)}
				>
					<TableIcon />
				</Button>
			</div>
			{#if tab === 'books' && selected.length}
				<div class="flex w-full flex-wrap items-center gap-2 border-t pt-3">
					<span class="text-sm text-muted-foreground">{countLabel(selected.length)} selected</span>
					<CrossPointSendButton mediaIds={selected} />
					<Button type="button" size="xs" variant="ghost" onclick={() => (selected = [])}>
						Clear selection
					</Button>
				</div>
			{/if}
		</div>

		<BookFilters {facets} onfacets={applyFacets} />

		{#if tab === 'series'}
			{#if seriesQuery.isPending}
				<div class="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
					{#each { length: 8 } as _, index (index)}
						<Skeleton class="aspect-[2/3] rounded-xl" />
					{/each}
				</div>
			{:else if seriesQuery.isError}
				<Alert variant="destructive">
					<AlertTitle>Unable to load series</AlertTitle>
					<AlertDescription>
						{seriesQuery.error instanceof Error ? seriesQuery.error.message : 'Request failed.'}
					</AlertDescription>
				</Alert>
			{:else if series.length === 0}
				<Empty class="rounded-xl border border-dashed bg-card">
					<EmptyHeader>
						<EmptyTitle>No series match</EmptyTitle>
						<EmptyDescription>
							Clear the filters, or scan the library if its folders are new.
						</EmptyDescription>
					</EmptyHeader>
				</Empty>
			{:else if view === 'grid'}
				<SeriesGrid {series} />
				<Pager page={seriesPage} noun="series" plural="series" onpage={(next) => navigate({ page: String(next) }, false)} />
			{:else}
				<SeriesTable {series} />
				<Pager page={seriesPage} noun="series" plural="series" onpage={(next) => navigate({ page: String(next) }, false)} />
			{/if}
		{:else if booksQuery.isPending}
			<Skeleton class="h-96 rounded-xl" />
		{:else if booksQuery.isError}
			<Alert variant="destructive">
				<AlertTitle>Unable to load books</AlertTitle>
				<AlertDescription>
					{booksQuery.error instanceof Error ? booksQuery.error.message : 'Request failed.'}
				</AlertDescription>
			</Alert>
		{:else if books.length === 0}
			<Empty class="rounded-xl border border-dashed bg-card">
				<EmptyHeader>
					<EmptyTitle>No books match</EmptyTitle>
					<EmptyDescription>
						Every filter is applied at once; clearing the format or tag usually helps.
					</EmptyDescription>
				</EmptyHeader>
			</Empty>
		{:else}
			<BookTable {books} bind:selected selectable showSeries onchange={() => booksQuery.refetch()} />
			<Pager page={booksPage} noun="book" onpage={(next) => navigate({ page: String(next) }, false)} />
		{/if}
	{/if}
</div>

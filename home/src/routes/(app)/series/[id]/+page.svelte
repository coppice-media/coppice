<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import SlidersHorizontalIcon from '@lucide/svelte/icons/sliders-horizontal';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Progress } from '@stump/ui/components/ui/progress';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { toast } from 'svelte-sonner';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleBooksDocument,
		ConsoleClearSeriesHistoryDocument,
		ConsoleFinishSeriesDocument,
		ConsoleSeriesDetailDocument
	} from '$lib/graphql/generated/graphql';
	import { countLabel, countNoun, minutesLabel } from '$lib/format';
	import { NO_FACETS, bookFilter, offsetPage } from '$lib/library';
	import BookTable from '$lib/components/library/BookTable.svelte';
	import CrossPointSendButton from '$lib/components/library/CrossPointSendButton.svelte';
	import Pager from '$lib/components/library/Pager.svelte';
	import SeriesReshapeSheet from '$lib/components/library/SeriesReshapeSheet.svelte';

	const PAGE_SIZE = 25;

	const seriesId = $derived(page.params.id ?? '');
	const pageNumber = $derived(Math.max(1, Number(page.url.searchParams.get('page') ?? '1') || 1));

	let selected = $state<string[]>([]);
	let reshapeOpen = $state(false);

	const queryClient = useQueryClient();

	const seriesQuery = createQuery(() => ({
		queryKey: ['series', seriesId],
		queryFn: () => request(ConsoleSeriesDetailDocument, { id: seriesId }),
		enabled: browser && !!seriesId
	}));
	const series = $derived(seriesQuery.data?.seriesById ?? null);

	const booksQuery = createQuery(() => ({
		queryKey: ['books', 'series', seriesId, pageNumber],
		queryFn: () =>
			request(ConsoleBooksDocument, {
				filter: bookFilter({ seriesId }, NO_FACETS),
				orderBy: [{ media: { field: 'NAME', direction: 'ASC' } }],
				pagination: { offset: { page: pageNumber, pageSize: PAGE_SIZE } }
			}),
		enabled: browser && !!seriesId
	}));
	const books = $derived(booksQuery.data?.media.nodes ?? []);
	const booksPage = $derived(offsetPage(booksQuery.data?.media.pageInfo));

	function refresh(): void {
		void queryClient.invalidateQueries({ queryKey: ['series', seriesId] });
		void booksQuery.refetch();
	}

	const finishSeries = createMutation(() => ({
		mutationFn: () => request(ConsoleFinishSeriesDocument, { id: seriesId }),
		onSuccess: (result) => {
			toast.success(`Marked ${countNoun(result.finishSeriesProgress, 'book')} as read.`);
			refresh();
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to mark the series as read.')
	}));
	const clearHistory = createMutation(() => ({
		mutationFn: () => request(ConsoleClearSeriesHistoryDocument, { id: seriesId }),
		onSuccess: (result) => {
			toast.success(`Cleared ${countNoun(result.clearSeriesReadingHistory, 'readthrough')}.`);
			refresh();
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to clear the history.')
	}));

	function turnPage(next: number): void {
		const url = new URL(page.url);
		url.searchParams.set('page', String(next));
		selected = [];
		void goto(url, { replaceState: true, noScroll: true, keepFocus: true });
	}
</script>

<svelte:head>
	<title>{series ? `${series.resolvedName} · Coppice` : 'Series · Coppice'}</title>
</svelte:head>

<div class="flex flex-col gap-6">
	{#if seriesQuery.isPending}
		<Skeleton class="h-28 rounded-xl" />
	{:else if seriesQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load this series</AlertTitle>
			<AlertDescription>
				{seriesQuery.error instanceof Error ? seriesQuery.error.message : 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else if !series}
		<Alert variant="destructive">
			<AlertTitle>Series not found</AlertTitle>
			<AlertDescription>It may have been merged away, deleted, or hidden from you.</AlertDescription>
		</Alert>
	{:else}
		<div class="flex flex-wrap items-start gap-4">
			<img
				class="hidden h-40 w-28 shrink-0 rounded-lg border bg-muted object-cover sm:block"
				src={series.thumbnail.url}
				alt=""
			/>
			<div class="mr-auto flex min-w-0 flex-col gap-2">
				<div class="flex flex-wrap items-center gap-2">
					<h1 class="text-2xl font-semibold tracking-tight">{series.resolvedName}</h1>
					{#if series.status !== 'READY'}
						<Badge variant="destructive">{series.status}</Badge>
					{/if}
					{#if series.sourceProvider}
						<Badge variant="secondary">Provider</Badge>
					{/if}
				</div>
				<p class="text-sm text-muted-foreground">
					<a
						class="hover:underline"
						href={resolve('/(app)/library/[id]', { id: series.library.id })}
					>
						{series.library.name}
					</a>
					· {countNoun(series.mediaCount, 'book')} · {series.unreadCount} unread ·
					{minutesLabel(Math.round(series.stats.totalReadingTimeSeconds / 60))} read
				</p>
				{#if series.metadata?.publisher || series.metadata?.year}
					<p class="text-sm text-muted-foreground">
						{[series.metadata?.publisher, series.metadata?.year].filter(Boolean).join(' · ')}
					</p>
				{/if}
				{#if series.resolvedDescription}
					<p class="max-w-2xl text-sm">{series.resolvedDescription}</p>
				{/if}
				<div class="max-w-sm">
					<Progress value={Math.round(series.percentageCompleted)} />
				</div>
				{#if series.tags.length}
					<div class="flex flex-wrap gap-1">
						{#each series.tags as tag (tag.id)}
							<Badge variant="outline">{tag.name}</Badge>
						{/each}
					</div>
				{/if}
			</div>
		</div>

		<div class="flex flex-wrap gap-2">
			<Button variant="outline" onclick={() => (reshapeOpen = true)}>
				<SlidersHorizontalIcon data-icon="inline-start" />
				Reshape
			</Button>
			<Button
				variant="outline"
				onclick={() => finishSeries.mutate()}
				disabled={finishSeries.isPending}
			>
				{finishSeries.isPending ? 'Marking…' : 'Mark all read'}
			</Button>
			<Button
				variant="outline"
				onclick={() => clearHistory.mutate()}
				disabled={clearHistory.isPending}
			>
				{clearHistory.isPending ? 'Clearing…' : 'Clear read history'}
			</Button>
			{#if selected.length}
				<span class="self-center text-sm text-muted-foreground">
					{countLabel(selected.length)} selected
				</span>
				<CrossPointSendButton mediaIds={selected} />
				<Button type="button" variant="ghost" onclick={() => (selected = [])}>Clear selection</Button>
			{/if}

		</div>
		{#if booksQuery.isPending}
			<Skeleton class="h-96 rounded-xl" />
		{:else if booksQuery.isError}
			<Alert variant="destructive">
				<AlertTitle>Unable to load the books</AlertTitle>
				<AlertDescription>
					{booksQuery.error instanceof Error ? booksQuery.error.message : 'Request failed.'}
				</AlertDescription>
			</Alert>
		{:else if books.length === 0}
			<Empty class="rounded-xl border border-dashed bg-card">
				<EmptyHeader>
					<EmptyTitle>No books in this series</EmptyTitle>
					<EmptyDescription>
						Scan the library, or move books in from another series with <em>Reshape</em>.
					</EmptyDescription>
				</EmptyHeader>
			</Empty>
		{:else}
			<BookTable {books} bind:selected selectable onchange={refresh} />
			<Pager page={booksPage} noun="book" onpage={turnPage} />
		{/if}

		<SeriesReshapeSheet
			bind:open={reshapeOpen}
			{series}
			{selected}
			onreshaped={(clearSelection: boolean) => {
				if (clearSelection) selected = [];
				refresh();
			}}
		/>
	{/if}
</div>

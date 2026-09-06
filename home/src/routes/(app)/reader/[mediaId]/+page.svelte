<script lang="ts">
	/**
	 * `/reader/[mediaId]` — the console's reader.
	 *
	 * EPUBs render with foliate-js over the server's Readium surfaces
	 * (`/api/v2/epub/{id}/manifest.json`, `positions.json`, `resource/{*path}`);
	 * comics and PDFs render as paged images over
	 * `/api/v2/media/{id}/page/{n}`. Both persist through the one unified
	 * reading-state mutation, `updateMediaProgress`, whose `oneOf` input picks
	 * the EPUB (Readium locator) or paged (page number) lane.
	 */
	import { browser } from '$app/environment';
	import { page as pageState } from '$app/state';
	import { createMutation, createQuery } from '@tanstack/svelte-query';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import { onDestroy } from 'svelte';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import {
		ReaderAnnotationsDocument,
		ReaderBookDocument,
		ReaderUpdateProgressDocument,
		ReaderVisiblePagesDocument,
		type MediaProgressInput
	} from '$lib/graphql/generated/graphql';
	import EpubReader from '$lib/components/reader/EpubReader.svelte';
	import PagedReader from '$lib/components/reader/PagedReader.svelte';
	import { decimal } from '$lib/components/reader/locator';

	const mediaId = $derived(pageState.params.mediaId ?? '');

	const bookQuery = createQuery(() => ({
		queryKey: ['readerBook', mediaId],
		queryFn: () => request(ReaderBookDocument, { id: mediaId }),
		enabled: browser && mediaId.length > 0
	}));
	const book = $derived(bookQuery.data?.mediaById);
	const isEpub = $derived(/epub/i.test(book?.extension ?? ''));

	const annotationsQuery = createQuery(() => ({
		queryKey: ['readerAnnotations', mediaId],
		queryFn: () => request(ReaderAnnotationsDocument, { id: mediaId }),
		enabled: browser && mediaId.length > 0
	}));
	const annotations = $derived(annotationsQuery.data?.annotationsByMediaId ?? []);

	// Duplicate-page marks renumber a book's pages; the visible list's length
	// is the page count the streaming route accepts.
	const visiblePagesQuery = createQuery(() => ({
		queryKey: ['readerVisiblePages', mediaId],
		queryFn: () => request(ReaderVisiblePagesDocument, { id: mediaId }),
		enabled: browser && mediaId.length > 0 && book !== undefined && !isEpub
	}));
	const pageCount = $derived(visiblePagesQuery.data?.mediaVisiblePages.length ?? 0);

	let saveError = $state<string | null>(null);
	let savedAt = $state<Date | null>(null);

	const progress = createMutation(() => ({
		mutationFn: (input: MediaProgressInput) =>
			request(ReaderUpdateProgressDocument, { id: mediaId, input }),
		onSuccess: () => {
			saveError = null;
			savedAt = new Date();
		},
		onError: (cause: unknown) => {
			saveError = cause instanceof Error ? cause.message : 'Progress could not be saved.';
		}
	}));

	// Relocations fire on every page turn, so writes are coalesced. Elapsed
	// time is sent as a delta the server accumulates onto the session, which
	// is what the reading-activity screen counts.
	const WRITE_DELAY_MS = 1200;
	const openedAt = Date.now();
	let reportedSeconds = 0;
	let pending: MediaProgressInput | null = null;
	let timer: ReturnType<typeof setTimeout> | null = null;

	function flush(): void {
		if (timer) {
			clearTimeout(timer);
			timer = null;
		}
		const input = pending;
		pending = null;
		if (!input) return;

		const total = Math.floor((Date.now() - openedAt) / 1000);
		const elapsedSecondsDelta = Math.max(0, total - reportedSeconds);
		reportedSeconds = total;

		progress.mutate(
			input.epub
				? { epub: { ...input.epub, elapsedSecondsDelta } }
				: { paged: { ...input.paged, elapsedSecondsDelta } }
		);
	}

	function schedule(input: MediaProgressInput): void {
		pending = input;
		if (timer) clearTimeout(timer);
		timer = setTimeout(flush, WRITE_DELAY_MS);
	}

	onDestroy(flush);
</script>

<svelte:head>
	<title>{book?.resolvedName ?? 'Reader'} · Stump</title>
</svelte:head>

<div class="flex flex-col gap-4">
	<div class="flex flex-wrap items-center gap-3">
		<Button variant="ghost" size="sm" onclick={() => history.back()}>
			<ArrowLeftIcon data-icon="inline-start" />
			Back
		</Button>
		<div class="mr-auto">
			<h1 class="text-xl font-semibold tracking-tight">
				{book?.resolvedName ?? 'Reader'}
			</h1>
			{#if book?.series}
				<p class="text-sm text-muted-foreground">{book.series.name}</p>
			{/if}
		</div>
		{#if progress.isPending}
			<span class="text-xs text-muted-foreground">Saving…</span>
		{:else if savedAt}
			<span class="text-xs text-muted-foreground">
				Progress saved {savedAt.toLocaleTimeString()}
			</span>
		{/if}
	</div>

	{#if saveError}
		<Alert variant="destructive">
			<AlertTitle>Progress not saved</AlertTitle>
			<AlertDescription>{saveError}</AlertDescription>
		</Alert>
	{/if}

	{#if bookQuery.isPending}
		<div class="flex flex-col gap-3">
			<Skeleton class="h-9 w-64" />
			<Skeleton class="h-[70vh] min-h-[420px] rounded-xl" />
		</div>
	{:else if bookQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load this book</AlertTitle>
			<AlertDescription>
				{bookQuery.error instanceof Error ? bookQuery.error.message : 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else if !book}
		<Empty class="rounded-xl border border-dashed bg-card">
			<EmptyHeader>
				<EmptyTitle>Book not found</EmptyTitle>
				<EmptyDescription>
					This book either does not exist or is not in a library you can read.
				</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else if isEpub}
		<EpubReader
			{mediaId}
			storedLocator={book.readProgress?.locator ?? null}
			storedPercentage={decimal(book.readProgress?.percentageCompleted)}
			{annotations}
			onLocator={({ locator, percentage, isComplete }) =>
				schedule({ epub: { locator, percentage, isComplete } })}
		/>
	{:else if visiblePagesQuery.isPending}
		<Skeleton class="h-[70vh] min-h-[420px] rounded-xl" />
	{:else if visiblePagesQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load the page list</AlertTitle>
			<AlertDescription>
				{visiblePagesQuery.error instanceof Error
					? visiblePagesQuery.error.message
					: 'Request failed.'}
			</AlertDescription>
		</Alert>
	{:else if pageCount === 0}
		<Empty class="rounded-xl border border-dashed bg-card">
			<EmptyHeader>
				<EmptyTitle>Nothing to read</EmptyTitle>
				<EmptyDescription>
					This file has no readable pages. It may be missing from disk or still awaiting analysis.
				</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else}
		<PagedReader
			{mediaId}
			{pageCount}
			startPage={book.readProgress?.page ?? 1}
			{annotations}
			onPage={(value) => schedule({ paged: { page: value } })}
		/>
	{/if}
</div>

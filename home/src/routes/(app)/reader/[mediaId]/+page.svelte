<script lang="ts">
	/**
	 * `/reader/[mediaId]` — the console's reader.
	 *
	 * EPUBs render with foliate-js over the server's Readium surfaces
	 * (`/api/v2/epub/{id}/manifest.json`, `positions.json`, `resource/{*path}`);
	 * comics and PDFs render as paged images over
	 * `/api/v2/media/{id}/page/{n}`; audiobooks stream their tracks from
	 * `/api/v2/media/{id}/audio/track/{n}`. All three persist through the one
	 * unified reading-state mutation, `updateMediaProgress`, whose `oneOf`
	 * input picks the EPUB (Readium locator), paged (page number), or audio
	 * (publication milliseconds) lane.
	 *
	 * Annotations: selecting EPUB text drafts a highlight, a paged book offers
	 * a note on the visible page, and an audiobook a note at the playhead; all
	 * save through `createAnnotation`. A recording has no Readium anchor, so a
	 * time note sends `positionMs` and no locator, and "Go to" seeks the
	 * player back to it.
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
		type MediaProgressInput,
		type ReadiumLocatorInput
	} from '$lib/graphql/generated/graphql';
	import AlsoAvailableAs from '$lib/components/reader/AlsoAvailableAs.svelte';
	import AudioReader from '$lib/components/reader/AudioReader.svelte';
	import EpubReader from '$lib/components/reader/EpubReader.svelte';
	import PagedReader from '$lib/components/reader/PagedReader.svelte';
	import ReaderAnnotations, {
		type ReaderAnnotationDraft,
		type ReaderAnnotationRecord
	} from '$lib/components/reader/ReaderAnnotations.svelte';
	import { decimal, type ReaderLocator } from '$lib/components/reader/locator';

	type EpubReaderHandle = { jumpToAnnotation(locator: ReaderLocator): void };
	type PagedReaderHandle = { jumpToPage(page: number): void };
	type AudioReaderHandle = {
		getCurrentPositionMs(): number;
		jumpToPosition(positionMs: number): void;
	};

	type PositionedAnnotation = ReaderAnnotationRecord & {
		annotationText: string | null;
		locator: ReaderLocator;
	};

	function locatorFor(annotation: ReaderAnnotationRecord): ReaderLocator | null {
		if (annotation.locator) return annotation.locator;
		if (!annotation.href && annotation.page === null) return null;
		return {
			chapterTitle: annotation.chapterTitle ?? '',
			href: annotation.href ?? '',
			title: null,
			type: 'application/xhtml+xml',
			locations: {
				fragments: annotation.fragment ? [annotation.fragment] : null,
				progression: null,
				position: annotation.page,
				totalProgression: annotation.progression,
				cssSelector: null,
				partialCfi: null
			},
			text: annotation.excerpt
				? { before: null, highlight: annotation.excerpt, after: null }
				: null
		};
	}

	const mediaId = $derived(pageState.params.mediaId ?? '');

	/**
	 * A deep link from the annotation hub or from an "Also available as" jump:
	 * `?href=…&fragment=…&progression=…&itemProgression=…` anchors an EPUB,
	 * `?page=…` a paged book, `?positionMs=…` a recording. `progression` is
	 * the whole-publication fraction (the hub's `progression`), so it selects
	 * a resource, never an in-resource offset; `itemProgression` is the
	 * fraction *inside* the resource, which is what a chapter-map conversion
	 * produces and the only way a mapped position lands mid-chapter.
	 *
	 * It decides where the reader opens and nothing else: the relocation it
	 * causes is not written back, so following a link — or opening the other
	 * edition at a converted, approximate position — never moves the book's
	 * saved position until the reader is actually paged.
	 */
	const deepLink = $derived.by(() => {
		const params = pageState.url.searchParams;
		const href = params.get('href');
		const fragment = params.get('fragment');
		const progression = params.has('progression')
			? Number(params.get('progression'))
			: Number.NaN;
		const itemProgression = params.has('itemProgression')
			? Number(params.get('itemProgression'))
			: Number.NaN;
		const startPage = Number.parseInt(params.get('page') ?? '', 10);
		const startPositionMs = Number.parseInt(params.get('positionMs') ?? '', 10);
		const percentage = Number.isFinite(progression) ? progression : undefined;
		const within = Number.isFinite(itemProgression) ? itemProgression : null;
		const page = Number.isFinite(startPage) ? startPage : undefined;
		const positionMs = Number.isFinite(startPositionMs) ? startPositionMs : undefined;
		if (
			!href &&
			!fragment &&
			percentage === undefined &&
			page === undefined &&
			positionMs === undefined
		)
			return null;
		return {
			locator: href
				? ({
						chapterTitle: '',
						href,
						title: null,
						type: 'application/xhtml+xml',
						locations: {
							fragments: fragment ? [fragment] : null,
							progression: within,
							position: page ?? null,
							totalProgression: percentage ?? null,
							cssSelector: null,
							partialCfi: null
						},
						text: null
					} satisfies ReaderLocator)
				: null,
			percentage,
			page,
			positionMs
		};
	});

	const bookQuery = createQuery(() => ({
		queryKey: ['readerBook', mediaId],
		queryFn: () => request(ReaderBookDocument, { id: mediaId }),
		enabled: browser && mediaId.length > 0
	}));
	const book = $derived(bookQuery.data?.mediaById);
	const isEpub = $derived(/epub/i.test(book?.extension ?? ''));
	// `Media.audio` is non-null exactly for audiobooks, so it is both the
	// discriminator and the whole shape the player needs.
	const isAudiobook = $derived(book?.audio != null);

	const annotationsQuery = createQuery(() => ({
		queryKey: ['readerAnnotations', mediaId],
		queryFn: () => request(ReaderAnnotationsDocument, { id: mediaId }),
		enabled: browser && mediaId.length > 0
	}));
	const nativeLocators = $derived(
		new Map(
			(annotationsQuery.data?.annotationsByMediaId ?? []).map((annotation) => [
				annotation.id,
				annotation.locator
			])
		)
	);
	const annotationRows = $derived(
		(annotationsQuery.data?.annotations.items ?? []).map((annotation) => ({
			...annotation,
			locator: nativeLocators.get(annotation.id) ?? null
		})) as ReaderAnnotationRecord[]
	);
	const positionedAnnotations = $derived.by((): PositionedAnnotation[] => {
		if (!book || isAudiobook) return [];
		return annotationRows.flatMap((annotation) => {
			const locator = locatorFor(annotation);
			const canPosition = isEpub
				? Boolean(locator?.href)
				: locator?.locations?.position !== null && locator?.locations?.position !== undefined;
			return canPosition && locator
				? [
						{
							...annotation,
							annotationText: annotation.excerpt ?? annotation.note,
							locator
						}
					]
				: [];
		});
	});
	// A recording's rows are reached by time, the other readers' by locator.
	const jumpableIds = $derived(
		new Set(
			isAudiobook
				? annotationRows.flatMap((annotation) =>
						annotation.positionMs !== null ? [annotation.id] : []
					)
				: positionedAnnotations.map((annotation) => annotation.id)
		)
	);

	// Duplicate-page marks renumber a book's pages; the visible list's length
	// is the page count the streaming route accepts. Neither an EPUB nor a
	// recording has pages at all, so neither asks.
	const visiblePagesQuery = createQuery(() => ({
		queryKey: ['readerVisiblePages', mediaId],
		queryFn: () => request(ReaderVisiblePagesDocument, { id: mediaId }),
		enabled: browser && mediaId.length > 0 && book !== undefined && !isEpub && !isAudiobook
	}));
	const pageCount = $derived(visiblePagesQuery.data?.mediaVisiblePages.length ?? 0);

	// The visible page a page note anchors to: the page the paged reader
	// opened at (clamped the same way) until it reports a turn. A turn is
	// keyed to the mounted reader, so a remount for another book or deep link
	// starts from its own opening page again.
	const pagedReaderKey = $derived(`${mediaId}${pageState.url.search}`);
	let turnedTo = $state<{ key: string; page: number } | null>(null);
	const currentPage = $derived(
		turnedTo?.key === pagedReaderKey
			? turnedTo.page
			: Math.min(
					Math.max(1, deepLink?.page ?? book?.readProgress?.page ?? 1),
					Math.max(1, pageCount)
				)
	);
	const notePage = $derived(
		book && !isEpub && !isAudiobook && pageCount > 0 ? currentPage : null
	);

	// Raw: the panel compares the draft it submitted against the live one.
	let annotationDraft = $state.raw<ReaderAnnotationDraft | null>(null);
	let readerArea = $state<HTMLDivElement | null>(null);
	let epubReader = $state<EpubReaderHandle | null>(null);
	let pagedReader = $state<PagedReaderHandle | null>(null);
	let audioReader = $state<AudioReaderHandle | null>(null);

	// The playhead a time note anchors to. `getCurrentPositionMs` reads the
	// player's own playhead state, so the "Note at" label follows playback
	// rather than the throttled progress reports.
	const noteTimeMs = $derived(
		isAudiobook && audioReader ? audioReader.getCurrentPositionMs() : null
	);

	function onSelection({ locator, excerpt }: { locator: ReadiumLocatorInput; excerpt: string }) {
		if (!excerpt.trim()) return;
		annotationDraft = {
			mediaId,
			locator,
			positionMs: null,
			excerpt,
			label: locator.chapterTitle || null
		};
	}

	function jumpToAnnotation(id: string): void {
		if (isAudiobook) {
			const positionMs = annotationRows.find((candidate) => candidate.id === id)?.positionMs;
			if (positionMs === null || positionMs === undefined) return;
			audioReader?.jumpToPosition(positionMs);
		} else {
			const annotation = positionedAnnotations.find((candidate) => candidate.id === id);
			if (!annotation) return;
			if (isEpub) {
				epubReader?.jumpToAnnotation(annotation.locator);
			} else {
				const target = annotation.page ?? annotation.locator.locations?.position;
				if (target !== null && target !== undefined) pagedReader?.jumpToPage(target);
			}
		}
		readerArea?.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}

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
	// A deep link opens the reader where the user has not read to, and the
	// renderer reports that opening — plus every layout settle after it — as a
	// relocation. `holdOpening` swallows those: writes resume on the first
	// relocation that reports a *different* position, which is the first real
	// page turn.
	let holdOpening = $state(false);
	let openingPosition: string | null = null;

	// Both renderers read their opening position untracked, so a second deep
	// link into the same book has to remount them; `search` is the only query
	// this route carries.
	const anchorKey = $derived(pageState.url.search);

	$effect(() => {
		// Re-armed for every deep link, including one that arrives while the
		// reader is already open.
		holdOpening = deepLink !== null;
		openingPosition = null;
	});

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

		// `MediaProgressInput` is an exclusive union of three branches, so each
		// one is narrowed by its own key: spreading an optional branch would
		// widen its required fields to `undefined`.
		if (input.epub) {
			progress.mutate({ epub: { ...input.epub, elapsedSecondsDelta } });
		} else if (input.paged) {
			progress.mutate({ paged: { ...input.paged, elapsedSecondsDelta } });
		} else if (input.audio) {
			progress.mutate({ audio: { ...input.audio, elapsedSecondsDelta } });
		}
	}

	function schedule(input: MediaProgressInput): void {
		if (holdOpening) {
			// The payload is a pure function of the position, so an identical
			// one is the same place, not a page turn.
			const position = JSON.stringify(input);
			if (openingPosition === null || openingPosition === position) {
				openingPosition = position;
				return;
			}
			holdOpening = false;
		}
		pending = input;
		if (timer) clearTimeout(timer);
		timer = setTimeout(flush, WRITE_DELAY_MS);
	}

	onDestroy(flush);
</script>

<svelte:head>
	<title>{book?.resolvedName ?? 'Reader'} · Coppice</title>
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

	{#if mediaId}
		<AlsoAvailableAs {mediaId} />
	{/if}

	{#if saveError}
		<Alert variant="destructive">
			<AlertTitle>Progress not saved</AlertTitle>
			<AlertDescription>{saveError}</AlertDescription>
		</Alert>
	{/if}

	<!-- `readerArea` is what "Go to" scrolls back to from the panel below. -->
	<div bind:this={readerArea} class="scroll-mt-4">
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
		{:else if book.audio}
			<!-- Keyed on the book rather than `anchorKey`: a recording has no
			in-resource anchor to re-seek to, and remounting a playing element
			because some unrelated query parameter changed would stop the audio. A
			`?positionMs=` jump from the other edition is therefore read once, at
			mount, exactly like a stored head. -->
			{#key mediaId}
				<AudioReader
					bind:this={audioReader}
					audio={book.audio}
					startPositionMs={deepLink?.positionMs ?? book.readProgress?.positionMs ?? 0}
					onPosition={({ positionMs, trackIndex, isComplete }) =>
						schedule({ audio: { positionMs, trackIndex, isComplete } })}
				/>
			{/key}
		{:else if isEpub}
			{#key anchorKey}
				<EpubReader
					bind:this={epubReader}
					{mediaId}
					storedLocator={deepLink?.locator ?? book.readProgress?.locator ?? null}
					storedPercentage={deepLink?.percentage ??
						decimal(book.readProgress?.percentageCompleted)}
					annotations={positionedAnnotations}
					{onSelection}
					onLocator={({ locator, percentage, isComplete }) =>
						schedule({ epub: { locator, percentage, isComplete } })}
				/>
			{/key}
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
						This file has no readable pages. It may be missing from disk or still awaiting
						analysis.
					</EmptyDescription>
				</EmptyHeader>
			</Empty>
		{:else}
			{#key anchorKey}
				<PagedReader
					bind:this={pagedReader}
					{mediaId}
					{pageCount}
					startPage={deepLink?.page ?? book.readProgress?.page ?? 1}
					annotations={positionedAnnotations}
					onPage={(value) => {
						turnedTo = { key: pagedReaderKey, page: value };
						schedule({ paged: { page: value } });
					}}
				/>
			{/key}
		{/if}
	</div>
	{#if book}
		<ReaderAnnotations
			{mediaId}
			annotations={annotationRows}
			bind:draft={annotationDraft}
			selectable={isEpub}
			{notePage}
			{pageCount}
			{noteTimeMs}
			durationMs={book.audio?.durationMs ?? 0}
			jumpable={jumpableIds}
			onjump={jumpToAnnotation}
		/>
	{/if}
</div>

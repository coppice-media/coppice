<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import Edit3Icon from '@lucide/svelte/icons/edit-3';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import HeadphonesIcon from '@lucide/svelte/icons/headphones';
	import HistoryIcon from '@lucide/svelte/icons/history';
	import HighlighterIcon from '@lucide/svelte/icons/highlighter';
	import LibraryBigIcon from '@lucide/svelte/icons/library-big';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import { request } from '@stump/ui/graphql/client';
	import {
		ApplyBookMetadataCandidateDocument,
		ApplyBookMetadataDocument,
		BookCoverDocument,
		BookDetailDocument,
		BookHighlightsDocument,
		BookMetadataCandidatesDocument,
		BookReadingLogDocument,
		DeleteBookReviewDocument,
		SimilarBooksDocument,
		UpsertBookReviewDocument
	} from '$lib/graphql/generated/graphql';
	import { SearchBookMetadataDocument, type MetadataSearchResult } from '@stump/ui/graphql/metadata';
	import { getHomeSession } from '$lib/session.svelte';
	import CrossPointSendButton from '$lib/components/library/CrossPointSendButton.svelte';
	import BookFilesPanel from '$lib/components/book/BookFilesPanel.svelte';
	import BookHighlightsPanel from '$lib/components/book/BookHighlightsPanel.svelte';
	import BookMetadataEditor, { type MetadataScope } from '$lib/components/book/BookMetadataEditor.svelte';
	import BookReadAloudPanel from '$lib/components/book/BookReadAloudPanel.svelte';
	import BookReadingLogPanel from '$lib/components/book/BookReadingLogPanel.svelte';
	import BookReviewPanel from '$lib/components/book/BookReviewPanel.svelte';
	import BookSimilarPanel from '$lib/components/book/BookSimilarPanel.svelte';
	import type {
		BookCandidate,
		BookDetail,
		BookHighlight,
		BookReadingLog,
		SimilarBook
	} from '$lib/book/detail';
	import { fieldLabel, valueLabel, formatBytes, labelKind, normalizeSummary } from '$lib/book/detail';
	type DetailTab = 'details' | 'edit' | 'files' | 'reading' | 'highlights';
	type CandidateRecord = { matchCandidates?: BookCandidate[] | null };
	type CandidateQueryResponse = { bookMetadataCandidates?: CandidateRecord | null };
	type HighlightResponse = { annotations?: { items?: BookHighlight[] | null } | null };
	type ApplyMetadataRequest = {
		scope: MetadataScope;
		selectedFields: string[];
		metadata: Record<string, unknown>;
	};
	type ReviewInput = { rating: number; content: string; isPrivate: boolean };

	const session = getHomeSession();
	const queryClient = useQueryClient();
	const mediaId = $derived(page.params.mediaId ?? '');
	const tab = $derived<DetailTab>(
		(['details', 'edit', 'files', 'reading', 'highlights'] as const).includes(page.url.searchParams.get('tab') as DetailTab)
			? (page.url.searchParams.get('tab') as DetailTab)
			: 'details'
	);

	const detailQuery = createQuery(() => ({
		queryKey: ['book-detail', mediaId],
		queryFn: () => request(BookDetailDocument, { mediaId }),
		enabled: browser && mediaId.length > 0
	}));
	const detail = $derived((detailQuery.data?.bookDetail ?? null) as BookDetail | null);
	const editions = $derived(detail?.editions ?? []);
	const activeEdition = $derived(editions.find((edition) => edition.kind === 'EBOOK') ?? editions[0] ?? null);
	const audioEdition = $derived(editions.find((edition) => edition.kind === 'AUDIOBOOK') ?? null);
	const metadata = $derived(activeEdition?.metadata ?? {});
	const summary = $derived(normalizeSummary(metadata.summary));
	const workMetadataRows = $derived(
		Object.entries(detail?.workMetadata?.metadata ?? {}).filter(
			([key]) => !['title', 'author', 'summary', 'description', 'lockedFields'].includes(key)
		)
	);
	const coverQuery = createQuery(() => ({
		queryKey: ['book-cover', mediaId],
		queryFn: () => request(BookCoverDocument, { id: mediaId }),
		enabled: browser && mediaId.length > 0
	}));
	const coverUrl = $derived(coverQuery.data?.mediaById?.thumbnail?.url ?? null);

	const readingLogQuery = createQuery(() => ({
		queryKey: ['book-reading-log', mediaId],
		queryFn: async () => (await request(BookReadingLogDocument, { mediaId })).bookReadingLog as BookReadingLog | null,
		enabled: browser && mediaId.length > 0 && tab === 'reading'
	}));

	const highlightMediaIds = $derived.by(() => {
		const ids = [mediaId, ...editions.map((edition) => edition.mediaId)].filter(Boolean);
		return [...new Set(ids)];
	});
	const highlightsQuery = createQuery(() => ({
		queryKey: ['book-highlights', highlightMediaIds.join(':')],
		queryFn: async () => {
			const responses = await Promise.all(
				highlightMediaIds.map((id) => request(BookHighlightsDocument, { mediaId: id }) as Promise<HighlightResponse>)
			);
			return responses;
		},
		enabled: browser && highlightMediaIds.length > 0 && tab === 'highlights'
	}));
	const highlights = $derived(
		(highlightsQuery.data ?? []).flatMap((response) => response.annotations?.items ?? [])
	);

	const similarQuery = createQuery(() => ({
		queryKey: ['book-similar', mediaId],
		queryFn: async () => (await request(SimilarBooksDocument, { mediaId, limit: 12 })).similarBooks as SimilarBook[],
		enabled: browser && mediaId.length > 0 && tab === 'details'
	}));
	const similarBooks = $derived(similarQuery.data ?? []);
	let reviewBusy = $state(false);
	let metadataBusy = $state(false);
	let candidateBusy = $state(false);

	const canEditMetadata = $derived(
		Boolean(session.user?.isServerOwner || session.user?.permissions.includes('EDIT_METADATA'))
	);
	const canReadCandidates = $derived(
		Boolean(session.user?.isServerOwner || session.user?.permissions.includes('METADATA_FETCH_RECORD_READ'))
	);
	const canManageCandidates = $derived(
		Boolean(session.user?.isServerOwner || session.user?.permissions.includes('METADATA_FETCH_RECORD_MANAGE'))
	);
	const candidatesQuery = createQuery(() => ({
		queryKey: ['book-metadata-candidates', mediaId],
		queryFn: () => request(BookMetadataCandidatesDocument, { mediaId }),
		enabled: browser && mediaId.length > 0 && tab === 'edit' && canReadCandidates
	}));
	let searchedCandidates = $state<BookCandidate[] | null>(null);
	const candidates = $derived(
		searchedCandidates ??
			(((candidatesQuery.data as CandidateQueryResponse | undefined)?.bookMetadataCandidates?.matchCandidates ?? []) as BookCandidate[])
	);

	const canEditReview = $derived(Boolean(session.user));

	function navigateTab(next: string): void {
		if (!(['details', 'edit', 'files', 'reading', 'highlights'] as string[]).includes(next)) return;
		const url = new URL(page.url);
		if (next === 'details') url.searchParams.delete('tab');
		else url.searchParams.set('tab', next);
		void goto(url, { replaceState: true, noScroll: true, keepFocus: true });
	}

	function invalidateDetail(): void {
		void queryClient.invalidateQueries({ queryKey: ['book-detail', mediaId] });
		void queryClient.invalidateQueries({ queryKey: ['book-cover', mediaId] });
	}

	async function saveReview(input: ReviewInput): Promise<void> {
		reviewBusy = true;
		try {
			await request(UpsertBookReviewDocument, { mediaId, input } as never);
			invalidateDetail();
			toast.success('Review saved.');
		} catch (reason) {
			throw reason instanceof Error ? reason : new Error('Unable to save review.');
		} finally {
			reviewBusy = false;
		}
	}

	async function deleteReview(): Promise<void> {
		reviewBusy = true;
		try {
			await request(DeleteBookReviewDocument, { mediaId } as never);
			invalidateDetail();
			toast.success('Review deleted.');
		} catch (reason) {
			toast.error(reason instanceof Error ? reason.message : 'Unable to delete review.');
		} finally {
			reviewBusy = false;
		}
	}

	async function searchMetadata(input: { title?: string; author?: string; isbn?: string; limit?: number }): Promise<void> {
		const result = await request(SearchBookMetadataDocument, { mediaId, search: input });
		searchedCandidates = ((result as MetadataSearchResult).searchBookMetadata?.matchCandidates ?? []) as unknown as BookCandidate[];
	}

	async function applyMetadata(input: ApplyMetadataRequest): Promise<void> {
		metadataBusy = true;
		try {
			await request(ApplyBookMetadataDocument, { mediaId, input } as never);
			searchedCandidates = null;
			invalidateDetail();
			void candidatesQuery.refetch();
			toast.success(`Applied selected fields to ${input.scope.toLowerCase()}.`);
		} finally {
			metadataBusy = false;
		}
	}

	async function applyCandidate(input: { candidateIndex: number; scope: MetadataScope; selectedFields: string[] }): Promise<void> {
		candidateBusy = true;
		try {
			await request(ApplyBookMetadataCandidateDocument, {
				mediaId,
				candidateIndex: input.candidateIndex,
				scope: input.scope,
				selectedFields: input.selectedFields
			} as never);
			searchedCandidates = null;
			invalidateDetail();
			void candidatesQuery.refetch();
			toast.success(`Applied candidate fields to ${input.scope.toLowerCase()}.`);
		} finally {
			candidateBusy = false;
		}
	}

	function mismatchValue(field: string, value: unknown): string {
		return field.toLowerCase() === 'summary' || field.toLowerCase() === 'description'
			? normalizeSummary(value)
			: valueLabel(value);
	}

	const metadataRows = $derived([
		['Title', metadata.title],
		['Sort title', metadata.titleSort],
		['Series', metadata.series],
		['Writers', metadata.writers],
		['Narrators', metadata.narrators],
		['Publisher', metadata.publisher],
		['Release year', metadata.year],
		['Language', metadata.language],
		['Format', metadata.format],
		['Page count', metadata.pageCount],
		['ISBN', metadata.identifierIsbn],
		['Genres', metadata.genres]
	] as [string, unknown][]);
</script>

<svelte:head>
	<title>{detail?.title ? `${detail.title} · Coppice` : 'Book · Coppice'}</title>
</svelte:head>

<div class="flex flex-col gap-6">
	{#if detailQuery.isPending}
		<div class="grid gap-6 lg:grid-cols-[13rem_1fr]"><Skeleton class="h-72 rounded-xl" /><div class="flex flex-col gap-3"><Skeleton class="h-10 w-2/3" /><Skeleton class="h-5 w-1/2" /><Skeleton class="h-24 rounded-xl" /></div></div>
	{:else if detailQuery.isError || !detail}
		<Alert variant="destructive">
			<AlertTitle>Book not found</AlertTitle>
			<AlertDescription>This book is not available in a library visible to your account.</AlertDescription>
		</Alert>
	{:else}
		<div class="flex flex-wrap items-start gap-5">
			<div class="flex h-56 w-40 shrink-0 items-center justify-center overflow-hidden rounded-xl border bg-muted shadow-sm sm:h-64 sm:w-44">
				{#if coverUrl}
					<img class="size-full object-cover" src={coverUrl} alt="" />
				{:else}
					<div class="flex flex-col items-center gap-2 p-4 text-center text-muted-foreground"><LibraryBigIcon class="size-10" aria-hidden="true" /><span class="text-xs">Cover unavailable</span></div>
				{/if}
			</div>
			<div class="flex min-w-0 flex-1 flex-col gap-3">
				<div class="flex flex-wrap items-center gap-2">
					<h1 class="mr-auto text-2xl font-semibold tracking-tight sm:text-3xl">{detail.title}</h1>
					<Badge variant="secondary">Merged work</Badge>
				</div>
				{#if detail.authors.length}<p class="text-sm text-muted-foreground">{detail.authors.join(', ')}</p>{/if}
				<p class="text-sm text-muted-foreground">{editions.length} edition{editions.length === 1 ? '' : 's'} · metadata and files stay edition-specific</p>
				<div class="flex flex-wrap gap-2">
					{#if activeEdition}<Button href={resolve('/(app)/reader/[mediaId]', { mediaId: activeEdition.mediaId })}>{activeEdition.kind === 'AUDIOBOOK' ? 'Listen' : 'Read'} <ArrowLeftIcon data-icon="inline-end" class="rotate-180" aria-hidden="true" /></Button>{/if}
					{#if audioEdition && activeEdition?.kind !== 'AUDIOBOOK'}<Button variant="outline" href={resolve('/(app)/reader/[mediaId]', { mediaId: audioEdition.mediaId })}><HeadphonesIcon data-icon="inline-start" aria-hidden="true" />Listen</Button>{/if}
					<CrossPointSendButton mediaId={mediaId} />
					{#if canEditMetadata}<Button variant="outline" onclick={() => navigateTab('edit')}><Edit3Icon data-icon="inline-start" aria-hidden="true" />Edit metadata</Button>{/if}
				</div>
			</div>
		</div>

		<div class="flex flex-wrap items-center gap-3 border-b">
			<Tabs.Root value={tab} onValueChange={navigateTab}>
				<Tabs.List aria-label="Book detail sections" class="max-w-full overflow-x-auto">
					<Tabs.Trigger value="details"><FileTextIcon data-icon="inline-start" aria-hidden="true" />Details</Tabs.Trigger>
					<Tabs.Trigger value="edit" disabled={!canEditMetadata}><Edit3Icon data-icon="inline-start" aria-hidden="true" />Edit metadata</Tabs.Trigger>
					<Tabs.Trigger value="files"><LibraryBigIcon data-icon="inline-start" aria-hidden="true" />Files & editions</Tabs.Trigger>
					<Tabs.Trigger value="reading"><HistoryIcon data-icon="inline-start" aria-hidden="true" />Reading log</Tabs.Trigger>
					<Tabs.Trigger value="highlights"><HighlighterIcon data-icon="inline-start" aria-hidden="true" />Highlights</Tabs.Trigger>
				</Tabs.List>
			</Tabs.Root>
		</div>

		{#if tab === 'details'}
			<div class="grid gap-6 xl:grid-cols-[minmax(0,1fr)_22rem]">
				<div class="flex min-w-0 flex-col gap-6">
					<Card>
						<CardHeader><CardTitle>About this book</CardTitle><CardDescription>Shared work context with edition metadata shown separately below.</CardDescription></CardHeader>
						<CardContent class="flex flex-col gap-5">
							{#if summary}<p class="whitespace-pre-wrap text-sm leading-7">{summary}</p>{:else}<p class="text-sm text-muted-foreground">No summary is available.</p>{/if}
							<dl class="grid gap-x-5 gap-y-3 sm:grid-cols-2">
								{#each metadataRows as [label, value] (label)}
									{#if value != null && value !== ''}<div><dt class="text-xs text-muted-foreground">{label}</dt><dd class="mt-0.5 text-sm">{valueLabel(value)}</dd></div>{/if}
								{/each}
							</dl>
							<div class="flex flex-wrap gap-2"><Badge variant="outline">Source: {String(metadata.metadataSource ?? 'Local metadata')}</Badge>{#if metadata.lockedFields?.length}<Badge variant="outline">{metadata.lockedFields.length} locked fields</Badge>{/if}</div>
						</CardContent>
					</Card>
					{#if detail.mismatches.length}
						<Card>
							<CardHeader><CardTitle>Metadata mismatches</CardTitle><CardDescription>Differences are shown for review; no value is mirrored silently between editions.</CardDescription></CardHeader>
							<CardContent class="grid gap-3">
								{#each detail.mismatches as mismatch (mismatch.field)}
									<article class="rounded-lg border p-3">
										<div class="flex flex-wrap items-center gap-2"><h3 class="mr-auto text-sm font-medium">{fieldLabel(mismatch.field)}</h3>{#if mismatch.resolved}<Badge variant="secondary">Resolved</Badge>{:else}<Badge variant="destructive">Needs review</Badge>{/if}</div>
										<dl class="mt-3 grid gap-x-4 gap-y-2 text-xs sm:grid-cols-3">
											<div><dt class="text-muted-foreground">Work</dt><dd class="max-h-40 overflow-auto whitespace-pre-wrap break-words">{mismatchValue(mismatch.field, mismatch.workValue)}</dd></div>
											<div><dt class="text-muted-foreground">Ebook</dt><dd class="max-h-40 overflow-auto whitespace-pre-wrap break-words">{mismatchValue(mismatch.field, mismatch.ebookValue)}</dd></div>
											<div><dt class="text-muted-foreground">Audiobook</dt><dd class="max-h-40 overflow-auto whitespace-pre-wrap break-words">{mismatchValue(mismatch.field, mismatch.audiobookValue)}</dd></div>
										</dl>
									</article>
								{/each}
							</CardContent>
						</Card>
					{/if}
					<BookReadAloudPanel {detail} readAloud={detail.readAloud} />
					<BookReviewPanel review={detail.review} canEdit={canEditReview} busy={reviewBusy} onsave={saveReview} ondelete={deleteReview} />
				</div>
				<div class="flex flex-col gap-6">
					{#if detail.workMetadata}
						<Card>
							<CardHeader><CardTitle>Work metadata</CardTitle><CardDescription>Shared identity fields are separate from each edition.</CardDescription></CardHeader>
							<CardContent class="flex flex-col gap-3">
								{#if detail.workMetadata.title}<p class="text-sm font-medium">{detail.workMetadata.title}</p>{/if}
								{#if detail.workMetadata.author}<p class="text-sm text-muted-foreground">{detail.workMetadata.author}</p>{/if}
								{#each workMetadataRows as [key, value] (key)}
									{#if value != null && value !== ''}<div class="border-t pt-2 text-xs"><span class="text-muted-foreground">{fieldLabel(key)}</span><p class="mt-0.5">{valueLabel(value)}</p></div>{/if}
								{/each}
								{#if detail.workMetadata.lockedFields?.length}<Badge variant="outline">{detail.workMetadata.lockedFields.length} locked work fields</Badge>{/if}
							</CardContent>
						</Card>
					{/if}
					<Card>
						<CardHeader><CardTitle>Edition overview</CardTitle><CardDescription>Each edition keeps its own file and metadata provenance.</CardDescription></CardHeader>
						<CardContent class="flex flex-col gap-3">
							{#each editions as edition (edition.mediaId)}
								<div class="rounded-lg border p-3"><div class="flex items-center gap-2"><Badge variant="secondary">{labelKind(edition.kind)}</Badge><span class="mr-auto truncate text-sm font-medium">{edition.title}</span></div><p class="mt-2 text-xs text-muted-foreground">{edition.file.extension.toUpperCase()} · {formatBytes(edition.file.size)} · {edition.file.status}</p></div>
							{/each}
						</CardContent>
					</Card>
					{#if similarQuery.isPending}<Skeleton class="h-56 rounded-xl" />{:else if similarQuery.isError}<Card><CardContent class="p-4 text-sm text-muted-foreground">Similar books are unavailable right now.</CardContent></Card>{:else}<BookSimilarPanel books={similarBooks} />{/if}
				</div>
			</div>
		{:else if tab === 'edit'}
			{#if canEditMetadata && activeEdition}
				<BookMetadataEditor
					edition={activeEdition}
					audiobookEdition={audioEdition}
					workMetadata={detail.workMetadata}
					candidates={candidates}
					canEdit={canEditMetadata}
					canSearch={canManageCandidates}
					canApplyCandidate={canManageCandidates}
					loading={candidatesQuery.isPending || candidateBusy || metadataBusy}
					onsearch={searchMetadata}
					onapply={applyMetadata}
					onapplyCandidate={applyCandidate}
				/>
			{:else}
				<Empty class="rounded-xl border border-dashed"><EmptyHeader><EmptyTitle>Metadata editing is unavailable</EmptyTitle><EmptyDescription>Your account does not have permission to edit this book, or no edition is available.</EmptyDescription></EmptyHeader></Empty>
			{/if}
		{:else if tab === 'files'}
			<BookFilesPanel {detail} />
		{:else if tab === 'reading'}
			{#if readingLogQuery.isPending}<Skeleton class="h-72 rounded-xl" />{:else if readingLogQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load reading history</AlertTitle><AlertDescription>Reading history could not be loaded for this work.</AlertDescription></Alert>{:else}<BookReadingLogPanel log={readingLogQuery.data} />{/if}
		{:else if tab === 'highlights'}
			{#if highlightsQuery.isPending}<Skeleton class="h-72 rounded-xl" />{:else if highlightsQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load highlights</AlertTitle><AlertDescription>Annotations could not be loaded for this work.</AlertDescription></Alert>{:else}<BookHighlightsPanel highlights={highlights} />{/if}
		{/if}
	{/if}
</div>

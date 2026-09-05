<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQueries, createQuery, keepPreviousData, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '$lib/components/ui/empty';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '$lib/components/ui/table';
	import ReworkDetailSheet from '$lib/components/ReworkDetailSheet.svelte';
	import { getEditorSession } from '$lib/editor/session.svelte';
	import { request } from '$lib/graphql/client';
	import {
		IngestMediaQualityScoreDocument,
		IngestProviderCatalogDocument,
		LibraryAnalysisJobDocument,
		LibraryMediaDocument,
		MatchLibraryMediaDocument,
		RunLibraryQualityDocument,
		type JobStatus
	} from '$lib/graphql/generated/graphql';

	const session = getEditorSession();
	const queryClient = useQueryClient();

	const PAGE_SIZE = 25;
	const TERMINAL_JOB_STATUSES: JobStatus[] = ['COMPLETED', 'CANCELLED', 'FAILED'];

	let currentPage = $state(1);
	let selected = $state<string[]>([]);
	let matchProviders = $state<string[]>([]);
	let activeJobs = $state<string[]>([]);
	let sheetOpen = $state(false);
	let sheetItemId = $state<string | null>(null);
	let sheetItemTitle = $state<string | null>(null);

	let libraryId = $derived(session.selectedLibraryId);

	const mediaQuery = createQuery(() => ({
		queryKey: ['library-media', libraryId, currentPage],
		queryFn: () =>
			request(LibraryMediaDocument, {
				filter: { series: { libraryId: { eq: libraryId } } },
				pagination: { offset: { page: currentPage, pageSize: PAGE_SIZE, zeroBased: false } },
			}),
		enabled: browser && Boolean(libraryId),
		placeholderData: keepPreviousData
	}));
	const providersQuery = createQuery(() => ({
		queryKey: ['provider-catalog', 'matchable'],
		queryFn: () => request(IngestProviderCatalogDocument, { includeDisabled: false }),
		enabled: browser
	}));

	let nodes = $derived(mediaQuery.data?.media.nodes ?? []);
	const scoreQueries = createQueries(() => ({
		queries: nodes.map((book) => ({
			queryKey: ['ingest-media-quality-scores', book.id],
			queryFn: () => request(IngestMediaQualityScoreDocument, { mediaId: book.id }),
			enabled: browser
		}))
	}));

	let scoreById = $derived.by(() => {
		const map = new Map<string, number | null>();
		nodes.forEach((book, index) => {
			map.set(book.id, scoreQueries[index]?.data?.ingestMediaQualityReport?.score ?? null);
		});
		return map;
	});
	let totalItems = $derived(
		mediaQuery.data?.media.pageInfo && 'totalItems' in mediaQuery.data.media.pageInfo
			? mediaQuery.data.media.pageInfo.totalItems
			: nodes.length
	);
	let totalPages = $derived(
		mediaQuery.data?.media.pageInfo && 'totalPages' in mediaQuery.data.media.pageInfo
			? Math.max(mediaQuery.data.media.pageInfo.totalPages, 1)
			: 1
	);
	let allSelected = $derived(nodes.length > 0 && nodes.every((book) => selected.includes(book.id)));
	let matchableProviders = $derived(
		(providersQuery.data?.ingestProviderCatalog ?? []).filter(
			(provider) =>
				provider.configured
				&& (provider.capabilities.includes('LOOKUP') || provider.capabilities.includes('IDENTIFY'))
		)
	);

	$effect(() => {
		if (!browser || !activeJobs.length) return;
		const ids = [...activeJobs];
		const timer = setInterval(() => {
			void (async () => {
				for (const id of ids) {
				const job = (await request(LibraryAnalysisJobDocument, { id })).ingestAnalysisJob;
				if (!job || !TERMINAL_JOB_STATUSES.includes(job.status)) continue;
					if (job.status === 'COMPLETED') {
						toast.success('Library analysis finished.');
					} else {
						toast.error(job.error ?? `Library analysis ${job.status.toLowerCase()}.`);
					}
					void queryClient.invalidateQueries({ queryKey: ['ingest-media-quality-scores'] });
					activeJobs = activeJobs.filter((pending) => pending !== id);
				}
			})();
		}, 3000);
		return () => clearInterval(timer);
	});

	const runQualityMutation = createMutation(() => ({
		mutationFn: (mediaIds: string[]) => request(RunLibraryQualityDocument, { mediaIds }),
		onSuccess: (data) => {
			activeJobs = [...activeJobs, data.runLibraryQuality.id];
			toast.success(`Quality checks queued for ${selected.length} book${selected.length === 1 ? '' : 's'}.`);
			selected = [];
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to queue quality checks.')
	}));
	const matchMutation = createMutation(() => ({
		mutationFn: (input: { mediaIds: string[]; providers: string[] | null }) =>
			request(MatchLibraryMediaDocument, { mediaIds: input.mediaIds, providers: input.providers }),
		onSuccess: (data) => {
			activeJobs = [...activeJobs, data.matchLibraryMedia.id];
			toast.success(`Provider matching queued for ${selected.length} book${selected.length === 1 ? '' : 's'}.`);
			selected = [];
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to queue provider matching.')
	}));

	function toggleAll(): void {
		selected = allSelected ? [] : nodes.map((book) => book.id);
	}
	function toggleRow(id: string): void {
		selected = selected.includes(id) ? selected.filter((entry) => entry !== id) : [...selected, id];
	}
	function toggleProvider(id: string): void {
		matchProviders = matchProviders.includes(id)
			? matchProviders.filter((entry) => entry !== id)
			: [...matchProviders, id];
	}
	function goToPage(page: number): void {
		currentPage = Math.min(Math.max(page, 1), totalPages);
		selected = [];
	}
	function review(id: string, title: string): void {
		sheetItemId = id;
		sheetItemTitle = title;
		sheetOpen = true;
	}
	function refreshScores(): void {
		void queryClient.invalidateQueries({ queryKey: ['ingest-media-quality-scores'] });
	}
</script>

<svelte:head><title>Library · Stump ingest</title></svelte:head>

<div class="flex flex-col gap-6">
	<div>
		<p class="text-sm font-medium text-primary">Library-wide rework</p>
		<h1 class="text-3xl font-semibold tracking-tight">Library</h1>
		<p class="mt-1 max-w-2xl text-muted-foreground">Run quality checks and provider matching against books already in this library, then review and apply metadata without a drop folder.</p>
	</div>

	<Card>
		<CardHeader>
			<CardTitle>Library books</CardTitle>
			<CardDescription>{totalItems} book{totalItems === 1 ? '' : 's'} · page {currentPage} of {totalPages}{selected.length ? ` · ${selected.length} selected` : ''}</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-col gap-4">
			{#if selected.length || matchableProviders.length}
				<div class="flex flex-col gap-2 rounded-lg border p-3">
					<div class="flex flex-wrap items-center gap-2">
						<Button size="sm" variant="outline" disabled={!selected.length || runQualityMutation.isPending} onclick={() => runQualityMutation.mutate(selected)}>
							Run quality checks{selected.length ? ` (${selected.length})` : ''}
						</Button>
						<Button size="sm" disabled={!selected.length || matchMutation.isPending} onclick={() => matchMutation.mutate({ mediaIds: selected, providers: matchProviders.length ? matchProviders : null })}>
							Match providers{selected.length ? ` (${selected.length})` : ''}
						</Button>
						{#if activeJobs.length}
							<Badge variant="secondary">{activeJobs.length} analysis job{activeJobs.length === 1 ? '' : 's'} running</Badge>
						{/if}
					</div>
					{#if matchableProviders.length}
						<div class="flex flex-wrap items-center gap-1">
							<span class="text-xs text-muted-foreground">Limit matching to:</span>
							{#each matchableProviders as provider (provider.id)}
								<button
									type="button"
									class="rounded-full border px-2.5 py-0.5 text-xs {matchProviders.includes(provider.id) ? 'border-primary bg-primary text-primary-foreground' : 'bg-background text-muted-foreground hover:bg-muted'}"
									aria-pressed={matchProviders.includes(provider.id)}
									onclick={() => toggleProvider(provider.id)}
								>
									{provider.name}
								</button>
							{/each}
							<span class="text-xs text-muted-foreground">no chips selected = all enabled providers</span>
						</div>
					{/if}
				</div>
			{/if}

			{#if !libraryId}
				<div class="p-6"><Empty><EmptyHeader><EmptyTitle>Select a library</EmptyTitle><EmptyDescription>Choose a library in the header to browse its books.</EmptyDescription></EmptyHeader></Empty></div>
			{:else if mediaQuery.isPending}
				<div class="flex flex-col gap-3">{#each Array(5) as _, index (index)}<Skeleton class="h-14 w-full" />{/each}</div>
			{:else if mediaQuery.isError}
				<Alert variant="destructive"><AlertTitle>Unable to load library books</AlertTitle><AlertDescription>{mediaQuery.error instanceof Error ? mediaQuery.error.message : 'The server did not return library books.'}</AlertDescription></Alert>
			{:else if !nodes.length}
				<div class="p-6"><Empty><EmptyHeader><EmptyTitle>No books in this library</EmptyTitle><EmptyDescription>Scan or stage books into the library to review them here.</EmptyDescription></EmptyHeader></Empty></div>
			{:else}
				<div class="overflow-x-auto">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead class="w-12"><Checkbox checked={allSelected} aria-label="Select all books on this page" onCheckedChange={() => toggleAll()} /></TableHead>
								<TableHead>Book</TableHead>
								<TableHead>Path</TableHead>
								<TableHead>Pages</TableHead>
								<TableHead>Quality score</TableHead>
								<TableHead class="text-right">Review</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{#each nodes as book (book.id)}
								<TableRow>
									<TableCell><Checkbox checked={selected.includes(book.id)} aria-label={`Select ${book.resolvedName}`} onCheckedChange={() => toggleRow(book.id)} /></TableCell>
									<TableCell class="max-w-[24rem]">
										<div class="flex items-center gap-3">
											<img src={book.thumbnail.url} alt="" loading="lazy" class="h-10 w-10 shrink-0 rounded object-cover" />
											<div class="min-w-0">
												<div class="truncate font-medium">{book.resolvedName}</div>
												<div class="truncate text-xs text-muted-foreground">{book.series.name}</div>
											</div>
										</div>
									</TableCell>
									<TableCell class="max-w-[20rem]"><div class="truncate text-xs text-muted-foreground">{book.path}</div></TableCell>
									<TableCell class="tabular-nums">{book.pages}</TableCell>
									<TableCell>
										{#if scoreById.get(book.id) === null || scoreById.get(book.id) === undefined}
											<span class="text-muted-foreground">—</span>
										{:else}
											<span class="tabular-nums font-medium {((scoreById.get(book.id) as number) < 70 ? 'text-destructive' : (scoreById.get(book.id) as number) < 90 ? 'text-amber-600' : 'text-emerald-600')}">{scoreById.get(book.id)}</span><span class="text-muted-foreground"> / 100</span>
										{/if}
									</TableCell>
									<TableCell><div class="flex justify-end"><Button size="sm" variant="outline" onclick={() => review(book.id, book.resolvedName)}>Open review</Button></div></TableCell>
								</TableRow>
							{/each}
						</TableBody>
					</Table>
				</div>
				<div class="flex items-center justify-between gap-2">
					<Button size="sm" variant="outline" disabled={currentPage <= 1 || mediaQuery.isPending} onclick={() => goToPage(currentPage - 1)}>Previous</Button>
					<span class="text-sm text-muted-foreground">Page {currentPage} of {totalPages}</span>
					<Button size="sm" variant="outline" disabled={currentPage >= totalPages || mediaQuery.isPending} onclick={() => goToPage(currentPage + 1)}>Next</Button>
				</div>
			{/if}
		</CardContent>
	</Card>

	<ReworkDetailSheet
		bind:open={sheetOpen}
		target="MEDIA"
		itemId={sheetItemId}
		itemTitle={sheetItemTitle}
		onApplied={refreshScores}
	/>
</div>

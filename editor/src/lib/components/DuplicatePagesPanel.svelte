<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@stump/ui/components/ui/table';
	import { request } from '@stump/ui/graphql/client';
	import {
		DuplicatePageCandidatesDocument,
		KnownDuplicatePagesDocument,
		MarkDuplicatePageDocument,
		UnmarkDuplicatePageDocument,
		type DuplicatePageAction
	} from '$lib/graphql/generated/graphql';

	let { libraryId }: { libraryId: string } = $props();

	const queryClient = useQueryClient();
	const DEFAULT_MIN_BOOKS = 3;
	const PREVIEW_LIMIT = 6;

	let minBooks = $state(DEFAULT_MIN_BOOKS);

	const candidatesQuery = createQuery(() => ({
		queryKey: ['duplicate-page-candidates', libraryId, minBooks],
		queryFn: () => request(DuplicatePageCandidatesDocument, { libraryId, minBooks, limit: null }),
		enabled: browser && Boolean(libraryId)
	}));
	const knownQuery = createQuery(() => ({
		queryKey: ['known-duplicate-pages', libraryId],
		queryFn: () => request(KnownDuplicatePagesDocument, { libraryId }),
		enabled: browser && Boolean(libraryId)
	}));

	let candidates = $derived(candidatesQuery.data?.duplicatePageCandidates ?? []);
	let known = $derived(knownQuery.data?.knownDuplicatePages ?? []);

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['duplicate-page-candidates', libraryId] });
		void queryClient.invalidateQueries({ queryKey: ['known-duplicate-pages', libraryId] });
		void queryClient.invalidateQueries({ queryKey: ['library-media'] });
	}

	const markMutation = createMutation(() => ({
		mutationFn: (input: { dhash: string; action: DuplicatePageAction }) =>
			request(MarkDuplicatePageDocument, { libraryId, dhash: input.dhash, action: input.action }),
		onSuccess: (data) => {
			toast.success(
				data.markDuplicatePage.action === 'SKIP'
					? 'Page hidden from every book in this library.'
					: 'Page kept; it will not be reported again.'
			);
			invalidate();
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to record the decision.')
	}));
	const unmarkMutation = createMutation(() => ({
		mutationFn: (dhash: string) => request(UnmarkDuplicatePageDocument, { libraryId, dhash }),
		onSuccess: () => {
			toast.success('Decision removed.');
			invalidate();
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to remove the decision.')
	}));

	function pageUrl(mediaId: string, visiblePage: number): string {
		return `/api/v2/media/${encodeURIComponent(mediaId)}/page/${visiblePage}`;
	}
	function onMinBooksInput(event: Event): void {
		const value = Number((event.currentTarget as HTMLInputElement).value);
		minBooks = Number.isFinite(value) && value >= 2 ? Math.floor(value) : DEFAULT_MIN_BOOKS;
	}
	function formatDate(value: string): string {
		return new Date(value).toLocaleString();
	}
</script>

<div class="flex flex-col gap-6">
	<Card>
		<CardHeader>
			<CardTitle>Duplicate page candidates</CardTitle>
			<CardDescription>
				Pages whose perceptual hash recurs across several books of this library, such as scanner credits or scraped filler. Skipping hides the page from every reader route without touching files; keeping records that it was reviewed.
			</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-col gap-4">
			<div class="flex flex-wrap items-end gap-3">
				<div class="flex flex-col gap-1">
					<Label for="duplicate-min-books">Minimum books</Label>
					<Input id="duplicate-min-books" type="number" min="2" step="1" class="w-28" value={minBooks} onchange={onMinBooksInput} />
				</div>
				<Button size="sm" variant="outline" disabled={candidatesQuery.isFetching} onclick={() => candidatesQuery.refetch()}>Refresh</Button>
				{#if candidates.length}
					<Badge variant="secondary">{candidates.length} group{candidates.length === 1 ? '' : 's'}</Badge>
				{/if}
			</div>

			{#if !libraryId}
				<div class="p-6"><Empty><EmptyHeader><EmptyTitle>Select a library</EmptyTitle><EmptyDescription>Choose a library in the header to review its duplicate pages.</EmptyDescription></EmptyHeader></Empty></div>
			{:else if candidatesQuery.isPending}
				<div class="flex flex-col gap-3">{#each Array(3) as _, index (index)}<Skeleton class="h-32 w-full" />{/each}</div>
			{:else if candidatesQuery.isError}
				<Alert variant="destructive"><AlertTitle>Unable to load duplicate pages</AlertTitle><AlertDescription>{candidatesQuery.error instanceof Error ? candidatesQuery.error.message : 'The server did not return duplicate page candidates.'}</AlertDescription></Alert>
			{:else if !candidates.length}
				<div class="p-6"><Empty><EmptyHeader><EmptyTitle>No unreviewed duplicate pages</EmptyTitle><EmptyDescription>Run the media analysis job to compute page hashes, or lower the minimum book count.</EmptyDescription></EmptyHeader></Empty></div>
			{:else}
				<div class="flex flex-col gap-4">
					{#each candidates as candidate (candidate.dhash)}
						<div class="flex flex-col gap-3 rounded-lg border p-3">
							<div class="flex flex-wrap items-center justify-between gap-2">
								<div class="flex flex-col">
									<span class="text-sm font-medium">{candidate.bookCount} books · {candidate.pageCount} pages</span>
									<span class="font-mono text-xs text-muted-foreground">{candidate.dhash}</span>
								</div>
								<div class="flex items-center gap-2">
									<Button size="sm" variant="outline" disabled={markMutation.isPending} onclick={() => markMutation.mutate({ dhash: candidate.dhash, action: 'KEEP' })}>Keep</Button>
									<Button size="sm" variant="destructive" disabled={markMutation.isPending} onclick={() => markMutation.mutate({ dhash: candidate.dhash, action: 'SKIP' })}>Skip everywhere</Button>
								</div>
							</div>
							<div class="flex flex-wrap gap-3">
								{#each candidate.occurrences.slice(0, PREVIEW_LIMIT) as occurrence (`${occurrence.mediaId}:${occurrence.page}`)}
									<figure class="flex w-28 flex-col gap-1">
										{#if occurrence.visiblePage != null}
											<img src={pageUrl(occurrence.mediaId, occurrence.visiblePage)} alt="" loading="lazy" class="h-36 w-28 rounded border object-cover" />
										{:else}
											<div class="flex h-36 w-28 items-center justify-center rounded border bg-muted text-xs text-muted-foreground">hidden</div>
										{/if}
										<figcaption class="flex flex-col text-xs">
											<span class="truncate" title={occurrence.mediaName}>{occurrence.mediaName}</span>
											<span class="text-muted-foreground">page {occurrence.page}</span>
										</figcaption>
									</figure>
								{/each}
								{#if candidate.occurrences.length > PREVIEW_LIMIT}
									<div class="flex h-36 w-28 items-center justify-center rounded border text-xs text-muted-foreground">+{candidate.occurrences.length - PREVIEW_LIMIT} more</div>
								{/if}
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</CardContent>
	</Card>

	<Card>
		<CardHeader>
			<CardTitle>Reviewed hashes</CardTitle>
			<CardDescription>Decisions already recorded for this library. Removing one makes the hash a candidate again and, for a skip, shows its pages again.</CardDescription>
		</CardHeader>
		<CardContent>
			{#if !libraryId}
				<p class="text-sm text-muted-foreground">Select a library to see its decisions.</p>
			{:else if knownQuery.isPending}
				<Skeleton class="h-24 w-full" />
			{:else if knownQuery.isError}
				<Alert variant="destructive"><AlertTitle>Unable to load decisions</AlertTitle><AlertDescription>{knownQuery.error instanceof Error ? knownQuery.error.message : 'The server did not return reviewed hashes.'}</AlertDescription></Alert>
			{:else if !known.length}
				<p class="text-sm text-muted-foreground">No decisions yet.</p>
			{:else}
				<Table stacked>
					<TableHeader>
						<TableRow>
							<TableHead>Hash</TableHead>
							<TableHead>Decision</TableHead>
							<TableHead>Recorded</TableHead>
							<TableHead class="text-right">Actions</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{#each known as entry (entry.dhash)}
							<TableRow>
								<TableCell class="font-mono text-xs break-all">{entry.dhash}</TableCell>
								<TableCell data-label="Decision"><Badge variant={entry.action === 'SKIP' ? 'destructive' : 'secondary'}>{entry.action === 'SKIP' ? 'Skipped' : 'Kept'}</Badge></TableCell>
								<TableCell data-label="Recorded" class="text-sm text-muted-foreground">{formatDate(entry.createdAt)}</TableCell>
								<TableCell>
									<div class="flex flex-wrap gap-1 @md/table:justify-end">
										<Button size="sm" variant="ghost" disabled={unmarkMutation.isPending} onclick={() => unmarkMutation.mutate(entry.dhash)}>Remove</Button>
									</div>
								</TableCell>
							</TableRow>
						{/each}
					</TableBody>
				</Table>
			{/if}
		</CardContent>
	</Card>
</div>

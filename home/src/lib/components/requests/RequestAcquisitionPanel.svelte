<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import InfoIcon from '@lucide/svelte/icons/info';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import * as Dialog from '@stump/ui/components/ui/dialog';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Progress } from '@stump/ui/components/ui/progress';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import {
		AcquisitionGrabsDocument,
		AcquisitionStatusDocument,
		CachedReleaseSearchDocument,
		GrabReleaseDocument,
		SearchReleasesDocument,
		type RequestFormat,
		type SearchReleasesQuery,
		type SearchReleasesQueryVariables
	} from '$lib/graphql/generated/graphql';
	import { absoluteTime, countNoun, decodeEntities, relativeTime } from '$lib/format';

	type ReleaseCandidate = SearchReleasesQuery['searchReleases']['candidates'][number];

	const TERMINAL_GRAB_PHASES = new Set(['completed', 'error', 'failed', 'cancelled']);
	const queryClient = useQueryClient();
	let {
		requestId,
		requestTitle,
		format,
		isbn = null
	}: {
		requestId: string;
		requestTitle: string;
		format: RequestFormat;
		isbn?: string | null;
	} = $props();

	let searchText = $state('');
	let selectedCandidate = $state<ReleaseCandidate | null>(null);

	const statusQuery = createQuery(() => ({
		queryKey: ['acquisition-status'],
		queryFn: () => request(AcquisitionStatusDocument, {}),
		enabled: browser
	}));
	const cachedSearchQuery = createQuery(() => ({
		queryKey: ['cached-release-search', requestId],
		queryFn: () => request(CachedReleaseSearchDocument, { requestId }),
		enabled: browser && !!requestId
	}));
	const grabsQuery = createQuery(() => ({
		queryKey: ['acquisition-grabs', requestId],
		queryFn: () => request(AcquisitionGrabsDocument, { requestId }),
		enabled: browser && !!requestId
	}));
	const grabs = $derived(grabsQuery.data?.acquisitionGrabs ?? []);
	const hasActiveGrabs = $derived(
		grabs.some((grab) => !TERMINAL_GRAB_PHASES.has(grab.phase.toLowerCase()))
	);
	const cachedSearch = $derived(cachedSearchQuery.data?.cachedReleaseSearch ?? null);

	const searchMutation = createMutation(() => ({
		mutationFn: (variables: SearchReleasesQueryVariables) =>
			request(SearchReleasesDocument, variables),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['cached-release-search', requestId] });
		}
	}));
	const releaseSearch = $derived(searchMutation.data?.searchReleases ?? cachedSearch);
	const candidates = $derived(
		(releaseSearch?.candidates ?? []).slice().sort((left, right) => right.score - left.score)
	);

	const grabMutation = createMutation(() => ({
		mutationFn: (torrentId: number) =>
			request(GrabReleaseDocument, { requestId, torrentId, confirm: true }),
		onSuccess: () => {
			selectedCandidate = null;
			void queryClient.invalidateQueries({ queryKey: ['acquisition-grabs', requestId] });
		}
	}));

	const bridgeStatus = $derived(statusQuery.data?.acquisitionStatus ?? null);
	const bridgeStatusLabel = $derived(
		statusQuery.isPending
			? 'Checking bridge…'
			: statusQuery.isError
				? 'Unreachable'
				: !bridgeStatus?.configured
					? 'Not configured'
					: !bridgeStatus.reachable
						? 'Unreachable'
						: bridgeStatus.ready
							? 'Ready'
							: 'Not ready'
	);
	const bridgeStatusVariant = $derived(
		bridgeStatusLabel === 'Ready'
			? 'secondary'
			: bridgeStatusLabel === 'Unreachable'
				? 'destructive'
				: 'outline'
	);

	const hasIsbnMatch = (candidate: ReleaseCandidate): boolean =>
		candidate.matchReasons.some((reason) => reason.toLowerCase() === 'isbn') ||
		(Boolean(isbn) && normalizeIsbn(candidate.isbn) === normalizeIsbn(isbn));

	function normalizeIsbn(value: string | null | undefined): string {
		return value?.replace(/[^a-z\d]/gi, '').toUpperCase() ?? '';
	}

	/** `Alice Smith · narrated by Bob Jones`, entity-decoded like the title. */
	function releasePeople(candidate: ReleaseCandidate): string {
		const authors = decodeEntities(candidate.authors.join(', ')) || 'Author not listed';
		const narrators = decodeEntities(candidate.narrators.join(', '));
		return narrators ? `${authors} · narrated by ${narrators}` : authors;
	}

	/** `Audiobooks - Philosophy` → `Philosophy`; the kind badge already says the rest. */
	function shortCategory(category: string): string {
		return decodeEntities(category.split(/\s+[-–—:]\s+/).at(-1) ?? category);
	}

	function releaseKindLabel(kind: string): string {
		const lower = kind.toLowerCase();
		return lower.charAt(0).toUpperCase() + lower.slice(1).replace(/_/g, ' ');
	}

	function findOnMam(): void {
		searchMutation.mutate({
			requestId,
			text: searchText.trim() || requestTitle,
			format,
			limit: 25
		});
	}

	function confirmGrab(): void {
		if (selectedCandidate) grabMutation.mutate(selectedCandidate.torrentId);
	}

	function progressPercent(progress: number): number {
		return Math.max(0, Math.min(100, progress * 100));
	}

	$effect(() => {
		if (!browser || !hasActiveGrabs) return;
		const interval = setInterval(() => void grabsQuery.refetch(), 30_000);
		return () => clearInterval(interval);
	});
</script>

<Card>
	<CardHeader>
		<CardTitle class="text-base">Acquisition</CardTitle>
		<CardDescription>Search MAM Bridge and explicitly add a release to the staged Editor queue.</CardDescription>
	</CardHeader>
	<CardContent class="flex flex-col gap-5">
		<div class="flex flex-wrap items-center gap-2 text-sm" role="status">
			<span class="text-muted-foreground">MAM bridge</span>
			<Badge variant={bridgeStatusVariant}>{bridgeStatusLabel}</Badge>
			{#if bridgeStatus?.mode}
				<span class="text-xs text-muted-foreground">{bridgeStatus.mode}</span>
			{/if}
			{#if bridgeStatus?.message}
				<span class="text-xs text-muted-foreground">{bridgeStatus.message}</span>
			{:else if statusQuery.isError}
				<span class="text-xs text-muted-foreground">{statusQuery.error instanceof Error ? statusQuery.error.message : 'Bridge status is unavailable.'}</span>
			{/if}
		</div>

		<div class="flex flex-col gap-2 sm:flex-row sm:items-end">
			<div class="flex min-w-0 flex-1 flex-col gap-2">
				<Label for="mam-release-search">Search text</Label>
				<Input id="mam-release-search" bind:value={searchText} placeholder={requestTitle} />
			</div>
			<Button onclick={findOnMam} disabled={searchMutation.isPending}>
				{searchMutation.isPending ? 'Searching…' : 'Find on MAM'}
			</Button>
		</div>

		{#if searchMutation.error}
			<p class="text-sm text-destructive" role="alert">
				{searchMutation.error instanceof Error ? searchMutation.error.message : 'MAM search failed.'}
			</p>
		{/if}

		{#if releaseSearch}
			<p class="text-sm text-muted-foreground">
				Last searched <span title={absoluteTime(releaseSearch.searchedAt)}>{relativeTime(releaseSearch.searchedAt)}</span>
				·
				<button type="button" class="font-medium text-foreground underline underline-offset-4" onclick={findOnMam} disabled={searchMutation.isPending}>Search again</button>
			</p>
			{#if releaseSearch.probe}
				<Alert>
					<InfoIcon aria-hidden="true" />
					<AlertTitle>Probe search</AlertTitle>
					<AlertDescription>First search after a bridge reset returns 5 results; the next search returns the full list.</AlertDescription>
				</Alert>
			{/if}
			{#if candidates.length}
				<Card size="sm" class="gap-0 py-0">
					<ul class="divide-y" aria-label="MAM releases">
						{#each candidates as candidate (candidate.torrentId)}
							{@const title = decodeEntities(candidate.title)}
							{@const people = releasePeople(candidate)}
							<li class="flex items-start gap-3 px-4 py-3">
								<div class="flex min-w-0 flex-1 flex-col gap-1.5">
									<div class="min-w-0">
										<p class="truncate text-sm font-medium" {title}>{title}</p>
										<p class="truncate text-xs text-muted-foreground" title={people}>{people}</p>
									</div>
									<div class="flex flex-wrap items-center gap-1.5">
										<Badge variant="secondary">{releaseKindLabel(candidate.kind)}</Badge>
										{#if candidate.fileType}<Badge variant="outline">{candidate.fileType}</Badge>{/if}
										{#if candidate.size}<Badge variant="outline">{candidate.size}</Badge>{/if}
										{#if candidate.numFiles != null}<Badge variant="outline">{countNoun(candidate.numFiles, 'file')}</Badge>{/if}
										{#if candidate.languageCode}<Badge variant="outline">{candidate.languageCode.toUpperCase()}</Badge>{/if}
										{#if candidate.categoryName}<Badge variant="outline">{shortCategory(candidate.categoryName)}</Badge>{/if}
										{#if candidate.freeleech}<Badge>Freeleech</Badge>{/if}
										{#if candidate.vip}<Badge variant="outline">VIP</Badge>{/if}
										{#if candidate.snatched}<Badge variant="outline">Snatched</Badge>{/if}
										{#if hasIsbnMatch(candidate)}<Badge variant="secondary">ISBN match</Badge>{/if}
										<span class="text-xs tabular-nums" aria-label="{candidate.seeders ?? 0} seeders, {candidate.leechers ?? 0} leechers">
											<span class="text-emerald-500">↑{candidate.seeders ?? 0}</span>
											<span class="text-destructive">↓{candidate.leechers ?? 0}</span>
										</span>
									</div>
									{#if candidate.matchReasons.length}
										<div class="flex flex-wrap gap-1" aria-label="Match reasons">
											{#each candidate.matchReasons as reason (reason)}
												<Badge variant="secondary" class="h-4 px-1.5 text-[10px] font-normal text-muted-foreground">{reason}</Badge>
											{/each}
										</div>
									{/if}
								</div>
								<div class="flex shrink-0 items-center gap-2">
									<Badge variant="outline" class="tabular-nums" title="Match score">{Math.round(candidate.score)}</Badge>
									<Button size="sm" variant="outline" onclick={() => (selectedCandidate = candidate)} disabled={grabMutation.isPending}>Add</Button>
								</div>
							</li>
						{/each}
					</ul>
				</Card>
			{:else}
				<Empty class="rounded-lg border border-dashed px-4 py-6">
					<EmptyHeader>
						<EmptyTitle>No MAM releases found</EmptyTitle>
						<EmptyDescription>Search again or adjust the search text.</EmptyDescription>
					</EmptyHeader>
				</Empty>
			{/if}
		{:else if cachedSearchQuery.isPending}
			<Skeleton class="h-24 rounded-lg" />
		{:else}
			<p class="text-sm text-muted-foreground">Search MAM Bridge to see available releases.</p>
		{/if}

		<section class="flex flex-col gap-3 border-t pt-4" aria-labelledby="acquisition-grabs-heading">
			<div class="flex items-center justify-between gap-3">
				<h3 id="acquisition-grabs-heading" class="text-sm font-semibold">Acquisition grabs</h3>
				{#if grabsQuery.isFetching}<span class="text-xs text-muted-foreground">Refreshing…</span>{/if}
			</div>
			{#if grabsQuery.isPending}
				<Skeleton class="h-20 rounded-lg" />
			{:else if grabsQuery.isError}
				<p class="text-sm text-destructive">{grabsQuery.error instanceof Error ? grabsQuery.error.message : 'Unable to load acquisition grabs.'}</p>
			{:else if grabs.length}
				<div class="flex flex-col gap-3">
					{#each grabs as grab (grab.id)}
						<Card class="bg-muted/15">
							<CardContent class="flex flex-col gap-3 p-4">
								<div class="flex flex-wrap items-start justify-between gap-2">
									<div class="min-w-0">
										<p class="font-medium">{grab.title}</p>
										<p class="mt-1 text-xs text-muted-foreground">{grab.phase} · Updated {relativeTime(grab.updatedAt)}</p>
									</div>
									<span class="text-xs tabular-nums text-muted-foreground">{Math.round(progressPercent(grab.progress))}%</span>
								</div>
								<Progress value={progressPercent(grab.progress)} aria-label={`Grab progress for ${grab.title}`} />
								{#if grab.error}<p class="text-sm text-destructive">{grab.error}</p>{/if}
								{#if grab.phase.toLowerCase() === 'completed' && grab.ingestItemId}
									<Button class="w-fit" size="sm" variant="outline" href="/editor/queue" target="_blank" rel="noopener noreferrer">Review in Editor</Button>
								{/if}
							</CardContent>
						</Card>
					{/each}
				</div>
			{:else}
				<p class="text-sm text-muted-foreground">No releases have been added for this request.</p>
			{/if}
		</section>
	</CardContent>
</Card>

<Dialog.Root
	open={selectedCandidate !== null}
	onOpenChange={(open) => {
		if (!open) selectedCandidate = null;
	}}
>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>Confirm release</Dialog.Title>
			<Dialog.Description>Confirm adding this MAM release to the acquisition bridge.</Dialog.Description>
		</Dialog.Header>
		{#if selectedCandidate}
			<dl class="grid gap-3 rounded-lg border bg-muted/25 p-4 text-sm">
				<div><dt class="text-muted-foreground">Title</dt><dd class="mt-1 font-medium">{decodeEntities(selectedCandidate.title)}</dd></div>
				<div><dt class="text-muted-foreground">Size</dt><dd class="mt-1 font-medium">{selectedCandidate.size || 'Unknown size'}</dd></div>
				<div><dt class="text-muted-foreground">Files</dt><dd class="mt-1 font-medium">{selectedCandidate.numFiles ?? 'Unknown'}</dd></div>
			</dl>
		{/if}
		{#if grabMutation.error}
			<p class="text-sm text-destructive" role="alert">{grabMutation.error instanceof Error ? grabMutation.error.message : 'Unable to add this release.'}</p>
		{/if}
		<Dialog.Footer>
			<Button variant="ghost" onclick={() => (selectedCandidate = null)}>Cancel</Button>
			<Button onclick={confirmGrab} disabled={!selectedCandidate || grabMutation.isPending}>
				{grabMutation.isPending ? 'Adding…' : 'Confirm add'}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

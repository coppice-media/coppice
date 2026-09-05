<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@stump/ui/components/ui/table';
	import ReworkDetailSheet from '$lib/components/ReworkDetailSheet.svelte';
	import StatusBadge from '$lib/components/StatusBadge.svelte';
	import { getEditorSession } from '$lib/editor/session.svelte';
	import { request } from '@stump/ui/graphql/client';
	import { IngestReworkItemsDocument } from '$lib/graphql/generated/graphql';
	import { formatBytes, humanize, offsetPagination } from '$lib/ingest/helpers';

	const session = getEditorSession();
	let minScore = $state(100);
	let selectedId = $state<string | null>(null);
	let detailOpen = $state(false);
	let libraryId = $derived(session.selectedLibraryId);

	const reworkQuery = createQuery(() => ({
		queryKey: ['rework-items', libraryId, minScore],
		queryFn: () => request(IngestReworkItemsDocument, { minScore, pagination: offsetPagination(100) }),
		enabled: browser && Boolean(libraryId)
	}));

	let rows = $derived(reworkQuery.data?.ingestReworkItems.nodes ?? []);
	let totalRows = $derived(
		reworkQuery.data?.ingestReworkItems.pageInfo && 'totalItems' in reworkQuery.data.ingestReworkItems.pageInfo
			? reworkQuery.data.ingestReworkItems.pageInfo.totalItems
			: rows.length
	);

	$effect(() => {
		const requested = page.url.searchParams.get('item');
		if (requested && requested !== selectedId) {
			selectedId = requested;
			detailOpen = true;
		}
	});

	function openItem(id: string): void {
		selectedId = id;
		detailOpen = true;
	}
</script>

<svelte:head><title>Rework · Stump ingest</title></svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div>
			<p class="text-sm font-medium text-primary">Quality review</p>
			<h1 class="text-3xl font-semibold tracking-tight">Rework</h1>
			<p class="mt-1 max-w-2xl text-muted-foreground">Start with the lowest score. Evidence and provider candidates stay visible until you choose what to apply.</p>
		</div>
		<label class="flex items-center gap-2 text-sm"><span class="text-muted-foreground">Maximum score</span><Input class="w-24" type="number" min="0" max="100" bind:value={minScore} /></label>
	</div>
	{#if !selectedId}
	<Card>
		<CardHeader><CardTitle>Items requiring attention</CardTitle><CardDescription>{totalRows} item{totalRows === 1 ? '' : 's'} ordered by score, creation time, and ID</CardDescription></CardHeader>
		<CardContent class="p-0">
			{#if !libraryId}
				<div class="p-6"><Empty><EmptyHeader><EmptyTitle>Select a library</EmptyTitle><EmptyDescription>Choose a library in the header to view quality reports.</EmptyDescription></EmptyHeader></Empty></div>
			{:else if reworkQuery.isPending}
				<div class="flex flex-col gap-3 p-6">{#each Array(5) as _, index (index)}<Skeleton class="h-14 w-full" />{/each}</div>
			{:else if reworkQuery.isError}
				<div class="p-6"><Alert variant="destructive"><AlertTitle>Unable to load rework items</AlertTitle><AlertDescription>{reworkQuery.error instanceof Error ? reworkQuery.error.message : 'The server did not return rework items.'}</AlertDescription></Alert></div>
			{:else if !rows.length}
				<div class="p-6"><Empty><EmptyHeader><EmptyTitle>No rework items</EmptyTitle><EmptyDescription>Items with scores at or below the selected threshold appear here.</EmptyDescription></EmptyHeader></Empty></div>
			{:else}
				<div class="overflow-x-auto">
					<Table>
						<TableHeader><TableRow><TableHead>Source</TableHead><TableHead>Score</TableHead><TableHead>Reasons</TableHead><TableHead>Status</TableHead><TableHead class="text-right">Review</TableHead></TableRow></TableHeader>
						<TableBody>
							{#each rows as row (row.item.id)}
								<TableRow>
									<TableCell class="max-w-[28rem]"><div class="truncate font-medium">{row.item.filename}</div><div class="text-xs text-muted-foreground">{formatBytes(row.item.sizeBytes)} · {row.item.mediaType}</div></TableCell>
									<TableCell><span class="text-2xl font-semibold tabular-nums">{row.item.qualityReport?.score ?? 0}</span><span class="text-muted-foreground"> / 100</span></TableCell>
									<TableCell class="max-w-sm"><div class="flex flex-wrap gap-1">{#each row.reasons as reason (reason.checkId)}<Badge variant={reason.status === 'FAIL' ? 'destructive' : 'secondary'}>{humanize(reason.checkId)}</Badge>{/each}</div></TableCell>
									<TableCell><StatusBadge status={row.item.status} /></TableCell>
									<TableCell><div class="flex justify-end"><Button size="sm" onclick={() => openItem(row.item.id)}>Open review</Button></div></TableCell>
								</TableRow>
							{/each}
						</TableBody>
					</Table>
				</div>
			{/if}
		</CardContent>
	</Card>
	{/if}

	<ReworkDetailSheet bind:open={detailOpen} itemId={selectedId} />
</div>

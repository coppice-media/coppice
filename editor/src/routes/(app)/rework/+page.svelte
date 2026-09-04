<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
	import * as Sheet from '$lib/components/ui/sheet';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '$lib/components/ui/empty';
	import { Input } from '$lib/components/ui/input';
	import { Separator } from '$lib/components/ui/separator';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '$lib/components/ui/table';
	import FieldPicker from '$lib/components/FieldPicker.svelte';
	import ProgressIndicator from '$lib/components/ProgressIndicator.svelte';
	import StatusBadge from '$lib/components/StatusBadge.svelte';
	import { getEditorSession } from '$lib/editor/session.svelte';
	import { request } from '$lib/graphql/client';
	import {
		ApproveIngestItemDocument,
		ApplyIngestMetadataDocument,
		IngestItemDocument,
		IngestReworkItemsDocument,
		RejectIngestItemDocument,
		RequeueIngestAnalysisDocument,
		type IngestMetadataFieldSelectionInput,
		type MergeStrategy
	} from '$lib/graphql/generated/graphql';
	import { formatBytes, humanize, offsetPagination, parseJsonObject } from '$lib/ingest/helpers';

	const session = getEditorSession();
	const queryClient = useQueryClient();
	let minScore = $state(100);
	let selectedId = $state<string | null>(null);
	let detailOpen = $state(false);
	let libraryId = $derived(session.selectedLibraryId);
	let queryItemId = $derived(selectedId);

	const reworkQuery = createQuery(() => ({
		queryKey: ['rework-items', libraryId, minScore],
		queryFn: () => request(IngestReworkItemsDocument, { minScore, pagination: offsetPagination(100) }),
		enabled: browser && Boolean(libraryId)
	}));
	const itemQuery = createQuery(() => ({
		queryKey: ['ingest-item', queryItemId],
		queryFn: () => request(IngestItemDocument, { id: queryItemId as string }),
		enabled: browser && Boolean(queryItemId)
	}));
	const applyMutation = createMutation(() => ({
		mutationFn: (selections: IngestMetadataFieldSelectionInput[]) =>
			request(ApplyIngestMetadataDocument, {
				input: { dropItemId: selectedId as string, selections, strategy: 'FILL_GAPS' as MergeStrategy }
			}),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['ingest-item', selectedId] });
			toast.success('Metadata selections applied.');
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to apply metadata selections.')
	}));
	const approveMutation = createMutation(() => ({
		mutationFn: () => request(ApproveIngestItemDocument, { dropItemId: selectedId as string, strategy: 'FILL_GAPS' }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
			toast.success('Item approved and committed.');
			detailOpen = false;
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to approve item.')
	}));
	const rejectMutation = createMutation(() => ({
		mutationFn: () => request(RejectIngestItemDocument, { dropItemId: selectedId as string, reason: 'Rejected during quality review' }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
			toast.success('Item rejected.');
			detailOpen = false;
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to reject item.')
	}));
	const requeueMutation = createMutation(() => ({
		mutationFn: () => request(RequeueIngestAnalysisDocument, { dropItemId: selectedId as string, force: true }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['analysis-queue'] });
			toast.success('Item requeued for a fresh analysis.');
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to requeue item.')
	}));

	let rows = $derived(reworkQuery.data?.ingestReworkItems.nodes ?? []);
	let item = $derived(itemQuery.data?.ingestItem ?? null);
	let totalRows = $derived(
		reworkQuery.data?.ingestReworkItems.pageInfo && 'totalItems' in reworkQuery.data.ingestReworkItems.pageInfo
			? reworkQuery.data.ingestReworkItems.pageInfo.totalItems
			: rows.length
	);
	let busy = $derived(applyMutation.isPending || approveMutation.isPending || rejectMutation.isPending || requeueMutation.isPending);

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

	function closeItem(): void {
		detailOpen = false;
	}

	function applyPicks(selections: IngestMetadataFieldSelectionInput[]): void {
		if (selectedId) applyMutation.mutate(selections);
	}

	function json(value: unknown): string {
		return JSON.stringify(value, null, 2);
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

	<Sheet.Root bind:open={detailOpen}>
		<Sheet.Content side="right" class="w-full overflow-y-auto sm:max-w-3xl">
			<Sheet.Header>
				<Sheet.Title>{item?.filename ?? 'Review item'}</Sheet.Title>
				<Sheet.Description>Inspect deterministic quality evidence, compare candidates, then apply fields or commit explicitly.</Sheet.Description>
			</Sheet.Header>
			{#if itemQuery.isPending}
				<div class="flex flex-col gap-3 py-6"><Skeleton class="h-7 w-3/4" /><Skeleton class="h-32 w-full" /><Skeleton class="h-64 w-full" /></div>
			{:else if itemQuery.isError}
				<Alert variant="destructive"><AlertTitle>Unable to load item</AlertTitle><AlertDescription>{itemQuery.error instanceof Error ? itemQuery.error.message : 'The server did not return this item.'}</AlertDescription></Alert>
			{:else if item}
				<div class="flex flex-col gap-6 py-6">
					<div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-4"><div><p class="font-medium">{item.filename}</p><p class="text-sm text-muted-foreground">{item.sourceSha256} · revision {item.revision}</p></div><div class="flex items-center gap-2"><StatusBadge status={item.status} /><ProgressIndicator dropItemId={item.id} analysisJobId={item.analysisJob?.id} compact /></div></div>
					<section aria-labelledby="quality-heading" class="flex flex-col gap-3">
						<div><h2 id="quality-heading" class="text-lg font-semibold">Quality report</h2><p class="text-sm text-muted-foreground">Algorithm {item.qualityReport?.algorithmVersion ?? 'not available'} · score {item.qualityReport?.score ?? 0}/100</p></div>
						{#if item.qualityReport?.checks.length}
							<div class="flex flex-col gap-2">{#each item.qualityReport.checks as check (check.checkId)}<div class="rounded-lg border p-3"><div class="flex flex-wrap items-center justify-between gap-2"><div class="font-medium">{check.label}</div><div class="flex items-center gap-2"><Badge variant={check.status === 'FAIL' ? 'destructive' : check.status === 'WARN' ? 'secondary' : 'outline'}>{humanize(check.status)}</Badge><span class="text-xs text-muted-foreground">weight {check.weight} · +{check.contribution.toFixed(1)}</span></div></div><p class="mt-2 text-sm text-muted-foreground">Normalized score {check.normalizedScore.toFixed(2)}</p><pre class="mt-2 max-h-32 overflow-auto rounded bg-muted p-2 text-xs">{json(check.evidence)}</pre></div>{/each}</div>
						{:else}
							<Empty><EmptyHeader><EmptyTitle>No quality report</EmptyTitle><EmptyDescription>Requeue this item to run deterministic checks.</EmptyDescription></EmptyHeader></Empty>
						{/if}
					</section>
					<Separator />
					<section aria-labelledby="candidates-heading" class="flex flex-col gap-3"><div><h2 id="candidates-heading" class="text-lg font-semibold">Provider candidates</h2><p class="text-sm text-muted-foreground">Candidates are evidence, not trusted metadata, until selected field by field.</p></div>{#if item.metadataCandidates.length}{#each item.metadataCandidates as candidate (candidate.id)}<div class="rounded-lg border p-3"><div class="flex flex-wrap items-center justify-between gap-2"><div class="font-medium">{candidate.provider} <span class="text-xs font-normal text-muted-foreground">v{candidate.providerVersion}{candidate.model ? ` · ${candidate.model}` : ''}</span></div><div class="flex items-center gap-2"><StatusBadge status={candidate.status} /><span class="text-sm tabular-nums">{Math.round(candidate.confidence * 100)}% confidence</span></div></div><pre class="mt-2 max-h-36 overflow-auto rounded bg-muted p-2 text-xs">{json(candidate.fields)}</pre>{#if candidate.fieldConfidences}<p class="mt-2 text-xs text-muted-foreground">Field confidence: {json(candidate.fieldConfidences)}</p>{/if}</div>{/each}{:else}<Empty><EmptyHeader><EmptyTitle>No provider candidates</EmptyTitle><EmptyDescription>Requeue the item after configuring a provider.</EmptyDescription></EmptyHeader></Empty>{/if}</section>
					<FieldPicker candidates={item.metadataCandidates} onsave={applyPicks} disabled={busy || item.status === 'COMMITTED'} />
					<div class="flex flex-wrap justify-end gap-2 border-t pt-4"><Button variant="outline" disabled={busy} onclick={() => requeueMutation.mutate()}>Requeue analysis</Button><Button variant="destructive" disabled={busy || item.status === 'COMMITTED' || item.status === 'REJECTED'} onclick={() => rejectMutation.mutate()}>Reject</Button><Button disabled={busy || item.status === 'COMMITTED' || item.status === 'REJECTED'} onclick={() => approveMutation.mutate()}>Approve and commit</Button></div>
				</div>
			{:else}
				<Empty><EmptyHeader><EmptyTitle>Item not found</EmptyTitle><EmptyDescription>The item may have been removed or committed in another session.</EmptyDescription></EmptyHeader></Empty>
			{/if}
		</Sheet.Content>
	</Sheet.Root>
</div>

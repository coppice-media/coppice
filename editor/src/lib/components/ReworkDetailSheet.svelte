<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '$lib/components/ui/empty';
	import * as Sheet from '$lib/components/ui/sheet';
	import { Separator } from '$lib/components/ui/separator';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import FieldPicker from '$lib/components/FieldPicker.svelte';
	import ProgressIndicator from '$lib/components/ProgressIndicator.svelte';
	import StatusBadge from '$lib/components/StatusBadge.svelte';
	import ProviderSearchDialog from '$lib/components/ProviderSearchDialog.svelte';
	import { request } from '$lib/graphql/client';
	import {
		ApproveIngestItemDocument,
		ApplyIngestMetadataDocument,
		IngestItemDocument,
		IngestMediaMetadataCandidatesDocument,
		IngestMediaQualityReportDocument,
		RejectIngestItemDocument,
		RequeueIngestAnalysisDocument,
		type IngestMetadataFieldSelectionInput,
		type MergeStrategy
	} from '$lib/graphql/generated/graphql';
	import { humanize } from '$lib/ingest/helpers';

	let {
		open = $bindable(false),
		itemId = $bindable(null),
		target = 'DROP_ITEM',
		itemTitle = null,
		onApplied
	}: {
		open?: boolean;
		itemId?: string | null;
		target?: 'DROP_ITEM' | 'MEDIA';
		itemTitle?: string | null;
		onApplied?: () => void;
	} = $props();

	const queryClient = useQueryClient();
	let searchOpen = $state(false);
	let queryItemId = $derived(itemId);
	let isMedia = $derived(target === 'MEDIA');

	const itemQuery = createQuery(() => ({
		queryKey: ['ingest-item', queryItemId],
		queryFn: () => request(IngestItemDocument, { id: queryItemId as string }),
		enabled: browser && !isMedia && Boolean(queryItemId)
	}));
	const mediaReportQuery = createQuery(() => ({
		queryKey: ['ingest-media-quality-report', queryItemId],
		queryFn: () => request(IngestMediaQualityReportDocument, { mediaId: queryItemId as string }),
		enabled: browser && isMedia && Boolean(queryItemId)
	}));
	const mediaCandidatesQuery = createQuery(() => ({
		queryKey: ['ingest-media-metadata-candidates', queryItemId],
		queryFn: () => request(IngestMediaMetadataCandidatesDocument, { mediaId: queryItemId as string }),
		enabled: browser && isMedia && Boolean(queryItemId)
	}));
	const applyMutation = createMutation(() => ({
		mutationFn: (selections: IngestMetadataFieldSelectionInput[]) =>
			request(ApplyIngestMetadataDocument, {
				input: isMedia
					? { mediaId: itemId as string, selections, strategy: 'FILL_GAPS' as MergeStrategy }
					: { dropItemId: itemId as string, selections, strategy: 'FILL_GAPS' as MergeStrategy }
			}),
		onSuccess: () => {
			if (isMedia) {
				void queryClient.invalidateQueries({ queryKey: ['ingest-media-quality-report', itemId] });
				void queryClient.invalidateQueries({ queryKey: ['ingest-media-metadata-candidates', itemId] });
				void queryClient.invalidateQueries({ queryKey: ['ingest-media-quality-scores'] });
			} else {
				void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
				void queryClient.invalidateQueries({ queryKey: ['ingest-item', itemId] });
			}
			toast.success('Metadata selections applied.');
			onApplied?.();
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to apply metadata selections.')
	}));
	const approveMutation = createMutation(() => ({
		mutationFn: () => request(ApproveIngestItemDocument, { dropItemId: itemId as string, strategy: 'FILL_GAPS' }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
			toast.success('Item approved and committed.');
			open = false;
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to approve item.')
	}));
	const rejectMutation = createMutation(() => ({
		mutationFn: () => request(RejectIngestItemDocument, { dropItemId: itemId as string, reason: 'Rejected during quality review' }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
			toast.success('Item rejected.');
			open = false;
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to reject item.')
	}));
	const requeueMutation = createMutation(() => ({
		mutationFn: () => request(RequeueIngestAnalysisDocument, { dropItemId: itemId as string, force: true }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['analysis-queue'] });
			toast.success('Item requeued for a fresh analysis.');
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to requeue item.')
	}));

	let item = $derived(itemQuery.data?.ingestItem ?? null);
	let report = $derived(
		isMedia ? mediaReportQuery.data?.ingestMediaQualityReport ?? null : item?.qualityReport ?? null
	);
	let candidates = $derived(
		isMedia
			? mediaCandidatesQuery.data?.ingestMediaMetadataCandidates ?? []
			: item?.metadataCandidates ?? []
	);
	let loading = $derived(
		isMedia ? mediaReportQuery.isPending || mediaCandidatesQuery.isPending : itemQuery.isPending
	);
	let loadError = $derived(isMedia ? (mediaReportQuery.error ?? mediaCandidatesQuery.error) : itemQuery.error);
	let busy = $derived(applyMutation.isPending || approveMutation.isPending || rejectMutation.isPending || requeueMutation.isPending);
	let displayTitle = $derived((isMedia ? itemTitle : item?.filename) ?? 'Review item');

	function applyPicks(selections: IngestMetadataFieldSelectionInput[]): void {
		if (itemId) applyMutation.mutate(selections);
	}

	function json(value: unknown): string {
		return JSON.stringify(value, null, 2);
	}
</script>

<Sheet.Root bind:open>
	<Sheet.Content side="right" class="w-full overflow-y-auto sm:max-w-3xl">
		<Sheet.Header>
			<Sheet.Title>{displayTitle}</Sheet.Title>
			<Sheet.Description>Inspect deterministic quality evidence, compare candidates, then apply fields or commit explicitly.</Sheet.Description>
		</Sheet.Header>
		{#if loading}
			<div class="flex flex-col gap-3 py-6"><Skeleton class="h-7 w-3/4" /><Skeleton class="h-32 w-full" /><Skeleton class="h-64 w-full" /></div>
		{:else if loadError}
			<Alert variant="destructive"><AlertTitle>Unable to load item</AlertTitle><AlertDescription>{loadError instanceof Error ? loadError.message : 'The server did not return this item.'}</AlertDescription></Alert>
		{:else if isMedia}
			<div class="flex flex-col gap-6 py-6">
				<div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-4"><div><p class="font-medium">{itemTitle ?? 'Library book'}</p><p class="text-sm text-muted-foreground">{itemId}</p></div></div>
				<section aria-labelledby="quality-heading" class="flex flex-col gap-3">
					<div><h2 id="quality-heading" class="text-lg font-semibold">Quality report</h2><p class="text-sm text-muted-foreground">Algorithm {report?.algorithmVersion ?? 'not available'} · score {report?.score ?? 0}/100</p></div>
					{#if report?.checks.length}
						<div class="flex flex-col gap-2">{#each report.checks as check (check.checkId)}<div class="rounded-lg border p-3"><div class="flex flex-wrap items-center justify-between gap-2"><div class="font-medium">{check.label}</div><div class="flex items-center gap-2"><Badge variant={check.status === 'FAIL' ? 'destructive' : check.status === 'WARN' ? 'secondary' : 'outline'}>{humanize(check.status)}</Badge><span class="text-xs text-muted-foreground">weight {check.weight} · +{check.contribution.toFixed(1)}</span></div></div><p class="mt-2 text-sm text-muted-foreground">Normalized score {check.normalizedScore.toFixed(2)}</p><pre class="mt-2 max-h-32 overflow-auto rounded bg-muted p-2 text-xs">{json(check.evidence)}</pre></div>{/each}</div>
					{:else}
						<Empty><EmptyHeader><EmptyTitle>No quality report</EmptyTitle><EmptyDescription>Run quality checks from the Library screen to generate one.</EmptyDescription></EmptyHeader></Empty>
					{/if}
				</section>
				<Separator />
				<section aria-labelledby="candidates-heading" class="flex flex-col gap-3"><div class="flex flex-wrap items-end justify-between gap-2"><div><h2 id="candidates-heading" class="text-lg font-semibold">Provider candidates</h2><p class="text-sm text-muted-foreground">Candidates are evidence, not trusted metadata, until selected field by field.</p></div></div>{#if candidates.length}{#each candidates as candidate (candidate.id)}<div class="rounded-lg border p-3"><div class="flex flex-wrap items-center justify-between gap-2"><div class="font-medium">{candidate.provider} <span class="text-xs font-normal text-muted-foreground">v{candidate.providerVersion}{candidate.model ? ` · ${candidate.model}` : ''}</span></div><div class="flex items-center gap-2"><StatusBadge status={candidate.status} /><span class="text-sm tabular-nums">{Math.round(candidate.confidence * 100)}% confidence</span></div></div><pre class="mt-2 max-h-36 overflow-auto rounded bg-muted p-2 text-xs">{json(candidate.fields)}</pre>{#if candidate.fieldConfidences}<p class="mt-2 text-xs text-muted-foreground">Field confidence: {json(candidate.fieldConfidences)}</p>{/if}</div>{/each}{:else}<Empty><EmptyHeader><EmptyTitle>No provider candidates</EmptyTitle><EmptyDescription>Run "Match providers" from the Library screen to fetch candidates.</EmptyDescription></EmptyHeader></Empty>{/if}</section>
				<FieldPicker candidates={candidates} onsave={applyPicks} disabled={busy} />
			</div>
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
				<section aria-labelledby="candidates-heading" class="flex flex-col gap-3"><div class="flex flex-wrap items-end justify-between gap-2"><div><h2 id="candidates-heading" class="text-lg font-semibold">Provider candidates</h2><p class="text-sm text-muted-foreground">Candidates are evidence, not trusted metadata, until selected field by field.</p></div><Button size="sm" variant="outline" disabled={!itemId} onclick={() => (searchOpen = true)}>Search providers</Button></div>{#if item.metadataCandidates.length}{#each item.metadataCandidates as candidate (candidate.id)}<div class="rounded-lg border p-3"><div class="flex flex-wrap items-center justify-between gap-2"><div class="font-medium">{candidate.provider} <span class="text-xs font-normal text-muted-foreground">v{candidate.providerVersion}{candidate.model ? ` · ${candidate.model}` : ''}</span></div><div class="flex items-center gap-2"><StatusBadge status={candidate.status} /><span class="text-sm tabular-nums">{Math.round(candidate.confidence * 100)}% confidence</span></div></div><pre class="mt-2 max-h-36 overflow-auto rounded bg-muted p-2 text-xs">{json(candidate.fields)}</pre>{#if candidate.fieldConfidences}<p class="mt-2 text-xs text-muted-foreground">Field confidence: {json(candidate.fieldConfidences)}</p>{/if}</div>{/each}{:else}<Empty><EmptyHeader><EmptyTitle>No provider candidates</EmptyTitle><EmptyDescription>Requeue the item after configuring a provider.</EmptyDescription></EmptyHeader></Empty>{/if}</section>
				<FieldPicker candidates={item.metadataCandidates} onsave={applyPicks} disabled={busy || item.status === 'COMMITTED'} />
				<div class="flex flex-wrap justify-end gap-2 border-t pt-4"><Button variant="outline" disabled={busy} onclick={() => requeueMutation.mutate()}>Requeue analysis</Button><Button variant="destructive" disabled={busy || item.status === 'COMMITTED' || item.status === 'REJECTED'} onclick={() => rejectMutation.mutate()}>Reject</Button><Button disabled={busy || item.status === 'COMMITTED' || item.status === 'REJECTED'} onclick={() => approveMutation.mutate()}>Approve and commit</Button></div>
			</div>
		{:else}
			<Empty><EmptyHeader><EmptyTitle>Item not found</EmptyTitle><EmptyDescription>The item may have been removed or committed in another session.</EmptyDescription></EmptyHeader></Empty>
		{/if}
		{#if !isMedia}
			<ProviderSearchDialog bind:open={searchOpen} dropItemId={itemId} />
		{/if}
	</Sheet.Content>
</Sheet.Root>

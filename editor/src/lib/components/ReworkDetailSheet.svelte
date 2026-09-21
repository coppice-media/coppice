<script lang="ts">
	import { getEditorSession } from '$lib/editor/session.svelte';
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import * as Sheet from '@stump/ui/components/ui/sheet';
	import { Separator } from '@stump/ui/components/ui/separator';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import AudioCard from '$lib/components/AudioCard.svelte';
	import DropGroupStrip from '$lib/components/DropGroupStrip.svelte';
	import MetadataFieldPicker from '@stump/ui/components/metadata/FieldPicker.svelte';
	import AudioShape, { type AudioFacts } from '$lib/components/editor/AudioShape.svelte';
	import ProgressIndicator from '$lib/components/ProgressIndicator.svelte';
	import QualityReportPanel from '$lib/components/QualityReportPanel.svelte';
	import StatusBadge from '$lib/components/StatusBadge.svelte';
	import ProviderSearchDialog from '$lib/components/ProviderSearchDialog.svelte';
	import {
		baselineFromItem,
		baselineFromMedia,
		buildSavePlan,
		publicationKindOfItem,
		publicationKindOfMedia,
		type EditorMedia,
		type SavePlan
	} from '$lib/editor/fields';
	import { request } from '@stump/ui/graphql/client';
	import {
		SearchBookMetadataDocument,
		metadataSearchCandidateToShared,
		type MetadataSearchInput
	} from '@stump/ui/graphql/metadata';
	import type { SharedCandidateLike, SharedSavePlan } from '@stump/ui/editor-fields';
	import {
		ApplyIngestMetadataDocument,
		ApproveIngestItemDocument,
		EditorMediaDocument,
		IngestItemDocument,
		IngestMediaMetadataCandidatesDocument,
		IngestMediaQualityReportDocument,
		RejectIngestItemDocument,
		RequeueIngestAnalysisDocument,
		SetEditorMediaTagsDocument,
		UpdateEditorMediaMetadataDocument,
		type IngestMetadataFieldSelectionInput,
		type MergeStrategy
	} from '$lib/graphql/generated/graphql';

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
	const session = getEditorSession();
	let canEditMetadata = $derived(
		Boolean(session.user?.isServerOwner || session.user?.permissions.includes('EDIT_METADATA'))
	);

	const queryClient = useQueryClient();
	let searchOpen = $state(false);
	let searchedMediaCandidates = $state<SharedCandidateLike[] | null>(null);
	let searchedMediaTarget = $state<string | null>(null);
	let sheetElement = $state<HTMLDivElement | null>(null);
	let queryItemId = $derived(itemId);
	let isMedia = $derived(target === 'MEDIA');
	$effect(() => {
		if (isMedia && searchedMediaTarget === queryItemId) return;
		searchedMediaTarget = isMedia ? queryItemId : null;
		searchedMediaCandidates = null;
	});

	const itemQuery = createQuery(() => ({
		queryKey: ['ingest-item', queryItemId],
		queryFn: () => request(IngestItemDocument, { id: queryItemId as string }),
		enabled: browser && !isMedia && Boolean(queryItemId)
	}));
	const mediaQuery = createQuery(() => ({
		queryKey: ['editor-media', queryItemId],
		queryFn: () => request(EditorMediaDocument, { id: queryItemId as string }),
		enabled: browser && isMedia && Boolean(queryItemId)
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
		mutationFn: (input: {
			target: 'DROP_ITEM' | 'MEDIA';
			id: string;
			selections: IngestMetadataFieldSelectionInput[];
		}) =>
			request(ApplyIngestMetadataDocument, {
				input:
					input.target === 'MEDIA'
						? {
								mediaId: input.id,
								selections: input.selections,
								strategy: 'PREFER_EXTERNAL' as MergeStrategy
							}
						: {
								dropItemId: input.id,
								selections: input.selections,
								strategy: 'PREFER_EXTERNAL' as MergeStrategy
							}
			})
	}));
	const setTagsMutation = createMutation(() => ({
		mutationFn: (input: { id: string; tags: string[] }) =>
			request(SetEditorMediaTagsDocument, { id: input.id, tags: input.tags })
	}));
	const updateMetadataMutation = createMutation(() => ({
		mutationFn: (input: { id: string; metadata: SavePlan['mediaInput'] }) =>
			request(UpdateEditorMediaMetadataDocument, { id: input.id, input: input.metadata ?? {} })
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
	let media = $derived(mediaQuery.data?.mediaById ?? null);
	let report = $derived(
		isMedia ? mediaReportQuery.data?.ingestMediaQualityReport ?? null : item?.qualityReport ?? null
	);
	let candidates = $derived(
		isMedia
			? searchedMediaCandidates ?? mediaCandidatesQuery.data?.ingestMediaMetadataCandidates ?? []
			: item?.metadataCandidates ?? []
	);
	let itemBaseline = $derived(item ? baselineFromItem(item) : null);
	let mediaBaseline = $derived(media ? baselineFromMedia(media) : null);
	let editorBaseline = $derived(isMedia ? mediaBaseline : itemBaseline?.draft ?? null);
	let editorKind = $derived(
		isMedia ? (media ? publicationKindOfMedia(media) : 'book') : item ? publicationKindOfItem(item) : 'book'
	);
	let lockedFields = $derived(media?.metadata?.lockedFields ?? []);
	let loading = $derived(
		isMedia
			? mediaQuery.isPending || mediaReportQuery.isPending || mediaCandidatesQuery.isPending
			: itemQuery.isPending
	);
	let loadError = $derived(
		isMedia
			? (mediaQuery.error ?? mediaReportQuery.error ?? mediaCandidatesQuery.error)
			: itemQuery.error
	);
	let busy = $derived(
		applyMutation.isPending
			|| setTagsMutation.isPending
			|| updateMetadataMutation.isPending
			|| approveMutation.isPending
			|| rejectMutation.isPending
			|| requeueMutation.isPending
	);
	let displayTitle = $derived(
		(isMedia ? media?.resolvedName ?? itemTitle : item?.filename) ?? 'Review item'
	);

	function audioFactsForMedia(audio: NonNullable<EditorMedia['audio']>): AudioFacts {
		return {
			durationMs: audio.durationMs,
			codec: audio.codec,
			bitrate: audio.bitrate,
			sampleRate: audio.sampleRate,
			channels: audio.channels,
			chapterSource: audio.chapterSource,
			tracks: audio.tracks.map((track) => ({
				label: `Track ${track.index + 1}`,
				durationMs: track.durationMs,
				startOffsetMs: track.startOffsetMs,
				byteSize: track.byteSize,
				detail: track.mime
			})),
			chapters: audio.chapters.map((chapter) => ({
				title: chapter.title,
				startMs: chapter.startMs,
				endMs: chapter.endMs
			})),
			cover: null,
			assembled: null
		};
	}

	async function searchMediaMetadata(input: MetadataSearchInput): Promise<void> {
		if (!isMedia || !itemId) throw new Error('Provider search is only available for a selected media record.');
		const result = await request(SearchBookMetadataDocument, { mediaId: itemId, search: input });
		searchedMediaCandidates = (result.searchBookMetadata?.matchCandidates ?? []).map(metadataSearchCandidateToShared);
	}

	async function saveSharedPlan(plan: SharedSavePlan): Promise<void> {
		const editorPlan = buildSavePlan({
			mode: plan.mode,
			fields: plan.fields as Parameters<typeof buildSavePlan>[0]['fields'],
			draft: plan.draft,
			baseline: plan.baseline,
			sources: plan.sources,
			lockedFields
		});
		await savePlan(editorPlan);
	}

	async function savePlan(plan: SavePlan): Promise<void> {
		if (!canEditMetadata) throw new Error('Metadata editing requires the EDIT_METADATA permission or server ownership.');
		if (!itemId) throw new Error('No item selected.');
		const id = itemId;
		if (plan.picks.length) {
			await applyMutation.mutateAsync({ target, id, selections: plan.picks });
		}
		if (isMedia && plan.tags) {
			await setTagsMutation.mutateAsync({ id, tags: plan.tags });
		}
		if (isMedia && plan.mediaInput) {
			await updateMetadataMutation.mutateAsync({ id, metadata: plan.mediaInput });
		}
		if (isMedia) {
			void queryClient.invalidateQueries({ queryKey: ['editor-media', id] });
			void queryClient.invalidateQueries({ queryKey: ['ingest-media-metadata-candidates', id] });
			void queryClient.invalidateQueries({ queryKey: ['ingest-media-quality-report', id] });
			void queryClient.invalidateQueries({ queryKey: ['ingest-media-quality-scores'] });
			void queryClient.invalidateQueries({ queryKey: ['library-media'] });
		} else {
			void queryClient.invalidateQueries({ queryKey: ['ingest-item', id] });
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
		}
		toast.success(`${plan.changed.length} metadata field${plan.changed.length === 1 ? '' : 's'} saved.`);
		onApplied?.();
	}
</script>


<Sheet.Root bind:open>
	<Sheet.Content
		bind:ref={sheetElement}
		side="right"
		class="w-full overflow-y-auto sm:max-w-3xl"
		onOpenAutoFocus={(event) => {
			event.preventDefault();
			sheetElement?.scrollTo({ top: 0 });
			sheetElement?.focus({ preventScroll: true });
		}}
	>
		<Sheet.Header>
			<Sheet.Title>{displayTitle}</Sheet.Title>
			<Sheet.Description>Inspect deterministic quality evidence, compare provider values, then save metadata or commit explicitly.</Sheet.Description>
		</Sheet.Header>
		{#if loading}
			<div class="flex flex-col gap-3 py-6"><Skeleton class="h-7 w-3/4" /><Skeleton class="h-32 w-full" /><Skeleton class="h-64 w-full" /></div>
		{:else if loadError}
			<Alert variant="destructive"><AlertTitle>Unable to load item</AlertTitle><AlertDescription>{loadError instanceof Error ? loadError.message : 'The server did not return this item.'}</AlertDescription></Alert>
		{:else if isMedia && media && editorBaseline}
			<div class="flex flex-col gap-6 py-6">
				<div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-4">
					<div><p class="font-medium">{media.resolvedName}</p><p class="text-sm text-muted-foreground">{media.path}</p></div>
					<StatusBadge status={media.audio ? 'AUDIOBOOK' : 'BOOK'} />
				</div>
				{#if media.audio}
					<AudioShape audio={audioFactsForMedia(media.audio)} />
				{/if}
				{#if media.metadata?.metadataSource || media.metadata?.metadataExternalId}
					<div class="rounded-lg border border-dashed p-3 text-sm">
						<p class="font-medium">Metadata provenance</p>
						<div class="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-muted-foreground">
							{#if media.metadata?.metadataSource}<span>Source: {media.metadata.metadataSource}</span>{/if}
							{#if media.metadata?.metadataExternalId}<span>External ID: {media.metadata.metadataExternalId}</span>{/if}
						</div>
						<p class="mt-1 text-xs text-muted-foreground">Provenance is maintained by the server and is read-only in this editor.</p>
					</div>
				{/if}
				{#if !canEditMetadata}
					<Alert><AlertTitle>Metadata is read-only</AlertTitle><AlertDescription>Your account needs EDIT_METADATA or server ownership to save metadata.</AlertDescription></Alert>
				{/if}
				<QualityReportPanel report={report} dropItemId={null} disabled={busy} />
				<Separator />
				<MetadataFieldPicker
					mode="media"
					kind={editorKind}
					baseline={editorBaseline}
					resetKey={media.id}
					lockedFields={lockedFields}
					candidates={candidates}
					onsave={saveSharedPlan}
					onsearch={searchMediaMetadata}
					disabled={busy || !canEditMetadata}
				/>
			</div>
		{:else if item && editorBaseline}
			<div class="flex flex-col gap-6 py-6">
				<div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-4"><div><p class="font-medium">{item.filename}</p><p class="text-sm text-muted-foreground">{item.sourceSha256} · revision {item.revision}</p></div><div class="flex items-center gap-2"><StatusBadge status={item.status} /><ProgressIndicator dropItemId={item.id} analysisJobId={item.analysisJob?.id} compact /></div></div>
				{#if item.sidecars.length}
					<p class="text-sm text-muted-foreground">Staged beside it: {item.sidecars.join(', ')}</p>
				{/if}
				{#if item.audio}
					<AudioCard audio={item.audio} />
				{/if}
				<QualityReportPanel report={item.qualityReport} dropItemId={item.id} disabled={busy || item.status === 'COMMITTED'} />
				<Separator />
				<div class="flex flex-wrap items-center justify-between gap-2">
					<div><h2 class="text-lg font-semibold">Staged metadata</h2><p class="text-sm text-muted-foreground">Provider candidates are compared inside the editor; search can add a fresh candidate set.</p></div>
					<Button size="sm" variant="outline" disabled={!itemId || busy} onclick={() => (searchOpen = true)}>Search providers</Button>
				</div>
				{#if !canEditMetadata}
					<Alert><AlertTitle>Metadata is read-only</AlertTitle><AlertDescription>Your account needs EDIT_METADATA or server ownership to save metadata.</AlertDescription></Alert>
				{/if}
				<MetadataFieldPicker
					mode="item"
					kind={editorKind}
					baseline={editorBaseline}
					resetKey={item.id}
					staged={itemBaseline?.staged}
					candidates={item.metadataCandidates}
					onsave={saveSharedPlan}
					disabled={busy || !canEditMetadata || item.status === 'COMMITTED'}
				/>
				<DropGroupStrip siblings={item.dropGroupSiblings} dropGroupId={item.dropGroupId} />
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

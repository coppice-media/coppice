<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '$lib/components/ui/alert';
	import { Button } from '$lib/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '$lib/components/ui/empty';
	import { Separator } from '$lib/components/ui/separator';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '$lib/components/ui/table';
	import StatusBadge from '$lib/components/StatusBadge.svelte';
	import ProgressIndicator from '$lib/components/ProgressIndicator.svelte';
	import { getEditorSession } from '$lib/editor/session.svelte';
	import { request, stageIngestUploads, type UploadFileInput } from '$lib/graphql/client';
	import { uuid } from '$lib/utils/uuid';
	import {
		DiscardIngestItemDocument,
		EnqueueIngestAnalysisDocument,
		IngestDropFolderAndItemsDocument,
		ScanIngestDropFolderDocument,
		type IngestDropItemStatus
	} from '$lib/graphql/generated/graphql';
	import { formatBytes, offsetPagination } from '$lib/ingest/helpers';

	const session = getEditorSession();
	const queryClient = useQueryClient();
	let libraryId = $derived(session.selectedLibraryId);
	let statusFilter = $state<IngestDropItemStatus | ''>('');
	let files = $state<UploadFileInput[]>([]);
	let fileInput = $state<HTMLInputElement | null>(null);
	let dragActive = $state(false);

	const dropQuery = createQuery(() => ({
		queryKey: ['drop-items', libraryId, statusFilter],
		queryFn: () =>
			request(IngestDropFolderAndItemsDocument, {
				libraryId,
				status: statusFilter || null,
				pagination: offsetPagination(50)
			}),
		enabled: browser && Boolean(libraryId)
	}));
	const uploadMutation = createMutation(() => ({
		mutationFn: (entries: UploadFileInput[]) =>
			stageIngestUploads({
				libraryId,
				files: entries,
				startAnalysis: true,
				idempotencyKey: uuid()
			}),
		onSuccess: (result) => {
				files = [];
				void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
				toast.success(
					`${result.items.length} file${result.items.length === 1 ? '' : 's'} staged${result.deduplicated ? `; ${result.deduplicated} duplicate${result.deduplicated === 1 ? '' : 's'} skipped` : ''}.`
				);
			},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Upload failed.')
	}));
	const scanMutation = createMutation(() => ({
		mutationFn: () => request(ScanIngestDropFolderDocument, { libraryId }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
			void queryClient.invalidateQueries({ queryKey: ['drop-folder'] });
			toast.success('Drop folder scan completed.');
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Drop folder scan failed.')
	}));
	const enqueueMutation = createMutation(() => ({
		mutationFn: (dropItemIds: string[]) =>
			request(EnqueueIngestAnalysisDocument, { input: { dropItemIds, force: false } }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
			void queryClient.invalidateQueries({ queryKey: ['analysis-queue'] });
			toast.success('Analysis queued.');
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to queue analysis.')
	}));
	const discardMutation = createMutation(() => ({
		mutationFn: ({ dropItemId, reason }: { dropItemId: string; reason: string }) =>
			request(DiscardIngestItemDocument, { dropItemId, reason }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
			void queryClient.invalidateQueries({ queryKey: ['drop-folder'] });
			toast.success('Item discarded.');
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to discard item.')
	}));

	let items = $derived(dropQuery.data?.ingestDropItems.nodes ?? []);
	let folder = $derived(dropQuery.data?.ingestDropFolder);
	let totalItems = $derived(
		dropQuery.data?.ingestDropItems.pageInfo && 'totalItems' in dropQuery.data.ingestDropItems.pageInfo
			? dropQuery.data.ingestDropItems.pageInfo.totalItems
			: items.length
	);

	function addFiles(event: Event): void {
		const target = event.currentTarget as HTMLInputElement;
		if (target.files) addFileList(target.files);
		target.value = '';
	}

	function addFileList(list: FileList): void {
		const next = Array.from(list).map((file) => ({
			file,
			relativePath: file.webkitRelativePath || file.name
		}));
		files = [...files, ...next.filter((entry) => !files.some((existing) => existing.file.name === entry.file.name && existing.file.size === entry.file.size))];
	}

	function dropFiles(event: DragEvent): void {
		event.preventDefault();
		dragActive = false;
		if (event.dataTransfer?.files) addFileList(event.dataTransfer.files);
	}

	function removeFile(index: number): void {
		files = files.filter((_file, current) => current !== index);
	}

	function stageFiles(): void {
		if (!files.length || uploadMutation.isPending) return;
		uploadMutation.mutate(files);
	}

	function enqueue(itemId: string): void {
		enqueueMutation.mutate([itemId]);
	}

	function discard(itemId: string): void {
		if (!window.confirm('Discard this staged source? The immutable staging copy will be removed.')) return;
		discardMutation.mutate({ dropItemId: itemId, reason: 'Discarded from ingest editor' });
	}
</script>

<svelte:head><title>Drop folder · Stump ingest</title></svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-start justify-between gap-4">
		<div>
			<p class="text-sm font-medium text-primary">Staged ingest</p>
			<h1 class="text-3xl font-semibold tracking-tight">Drop folder</h1>
			<p class="mt-1 max-w-2xl text-muted-foreground">Stage source files first. Analysis and commit stay explicit, so the library never changes by surprise.</p>
		</div>
		<Button variant="outline" disabled={!libraryId || scanMutation.isPending} onclick={() => scanMutation.mutate()}>
			{scanMutation.isPending ? 'Scanning…' : 'Scan drop folder'}
		</Button>
	</div>

	{#if !libraryId}
		<Empty>
			<EmptyHeader>
				<EmptyTitle>Select a library</EmptyTitle>
				<EmptyDescription>Choose a library in the header before staging or reviewing files.</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else}
		<div class="grid gap-6 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
			<Card>
				<CardHeader>
					<CardTitle>Stage uploads</CardTitle>
					<CardDescription>Files are streamed to server-owned staging and hashed before admission.</CardDescription>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					<div
						role="button"
						tabindex="0"
						class="flex min-h-44 cursor-pointer flex-col items-center justify-center rounded-xl border-2 border-dashed p-6 text-center transition-colors hover:border-primary/60 hover:bg-muted/40 {dragActive ? 'border-primary bg-primary/5' : ''}"
						onclick={() => fileInput?.click()}
						onkeydown={(event) => {
							if (event.key === 'Enter' || event.key === ' ') fileInput?.click();
						}}
						ondragover={(event) => { event.preventDefault(); dragActive = true; }}
						ondragleave={() => (dragActive = false)}
						ondrop={dropFiles}
					>
						<p class="font-medium">Drop files or a folder here</p>
						<p class="mt-1 text-sm text-muted-foreground">EPUB, CBZ, CBR, PDF, and other supported formats</p>
						<input
							class="sr-only"
							bind:this={fileInput}
							type="file"
							multiple
							webkitdirectory
							onchange={addFiles}
						/>
					</div>
					{#if files.length}
						<div class="flex flex-col gap-2">
							<div class="flex items-center justify-between text-sm font-medium"><span>{files.length} selected</span><Button variant="ghost" size="sm" onclick={() => (files = [])}>Clear</Button></div>
							{#each files as entry, index (entry.file.name + entry.file.size)}
								<div class="flex items-center gap-3 rounded-md border px-3 py-2 text-sm">
									<span class="min-w-0 flex-1 truncate">{entry.relativePath}</span>
									<span class="shrink-0 text-muted-foreground">{formatBytes(entry.file.size)}</span>
									<Button variant="ghost" size="sm" aria-label={`Remove ${entry.file.name}`} onclick={() => removeFile(index)}>Remove</Button>
								</div>
							{/each}
						</div>
					{/if}
					<Button disabled={!files.length || uploadMutation.isPending} onclick={stageFiles}>
						{uploadMutation.isPending ? 'Staging files…' : `Stage ${files.length || ''} file${files.length === 1 ? '' : 's'}`}
					</Button>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>Folder status</CardTitle>
					<CardDescription>Watcher-discovered files and manually staged uploads share this queue.</CardDescription>
				</CardHeader>
				<CardContent>
					{#if dropQuery.isPending}
						<div class="flex flex-col gap-3"><Skeleton class="h-5 w-3/4" /><Skeleton class="h-5 w-1/2" /><Skeleton class="h-5 w-2/3" /></div>
					{:else if dropQuery.isError}
						<Alert variant="destructive"><AlertTitle>Unable to read drop folder</AlertTitle><AlertDescription>{dropQuery.error instanceof Error ? dropQuery.error.message : 'The server did not return folder status.'}</AlertDescription></Alert>
					{:else if folder}
						<div class="grid gap-4 sm:grid-cols-3">
							<div><p class="text-xs uppercase tracking-wide text-muted-foreground">Path</p><p class="mt-1 break-all font-medium">{folder.displayPath}</p></div>
							<div><p class="text-xs uppercase tracking-wide text-muted-foreground">Pending files</p><p class="mt-1 text-2xl font-semibold tabular-nums">{folder.pendingCount}</p></div>
							<div><p class="text-xs uppercase tracking-wide text-muted-foreground">Last discovered</p><p class="mt-1 font-medium">{folder.lastDiscoveredAt ? new Date(folder.lastDiscoveredAt).toLocaleString() : 'Not discovered yet'}</p></div>
						</div>
					{:else}
						<Empty><EmptyHeader><EmptyTitle>Drop folder is unavailable</EmptyTitle><EmptyDescription>This library has no configured staged-ingest folder.</EmptyDescription></EmptyHeader></Empty>
					{/if}
				</CardContent>
			</Card>
		</div>

		<Separator />
		<div class="flex flex-wrap items-end justify-between gap-4">
			<div><h2 class="text-xl font-semibold">Staged items</h2><p class="text-sm text-muted-foreground">{totalItems} item{totalItems === 1 ? '' : 's'} in this library</p></div>
			<label class="flex items-center gap-2 text-sm"><span class="text-muted-foreground">Status</span><select class="h-9 rounded-md border bg-background px-3" value={statusFilter} onchange={(event) => (statusFilter = (event.currentTarget as HTMLSelectElement).value as IngestDropItemStatus | '')}><option value="">All statuses</option><option value="RECEIVED">Received</option><option value="STAGED">Staged</option><option value="ANALYZING">Analyzing</option><option value="AWAITING_REVIEW">Awaiting review</option><option value="READY">Ready</option><option value="COMMITTED">Committed</option><option value="FAILED">Failed</option></select></label>
		</div>
		{#if dropQuery.isPending}
			<div class="grid gap-3">{#each Array(4) as _, index (index)}<Skeleton class="h-16 w-full" />{/each}</div>
		{:else if dropQuery.isError}
			<Alert variant="destructive"><AlertTitle>Unable to load staged items</AlertTitle><AlertDescription>{dropQuery.error instanceof Error ? dropQuery.error.message : 'The server did not return staged items.'}</AlertDescription></Alert>
		{:else if !items.length}
			<Empty><EmptyHeader><EmptyTitle>No staged items</EmptyTitle><EmptyDescription>Upload files above or run a folder scan to begin a staged ingest.</EmptyDescription></EmptyHeader></Empty>
		{:else}
			<div class="overflow-hidden rounded-xl border bg-background">
				<Table>
					<TableHeader><TableRow><TableHead>Source</TableHead><TableHead>Size</TableHead><TableHead>Status</TableHead><TableHead>Progress</TableHead><TableHead class="text-right">Actions</TableHead></TableRow></TableHeader>
					<TableBody>
						{#each items as item (item.id)}
							<TableRow>
								<TableCell class="max-w-[28rem]"><div class="truncate font-medium">{item.filename}</div><div class="truncate text-xs text-muted-foreground">{item.relativePath ?? item.filename}</div></TableCell>
								<TableCell class="whitespace-nowrap text-muted-foreground">{formatBytes(item.sizeBytes)}</TableCell>
								<TableCell><StatusBadge status={item.status} /></TableCell>
								<TableCell class="min-w-36"><ProgressIndicator dropItemId={item.id} analysisJobId={item.analysisJob?.id} compact /></TableCell>
								<TableCell><div class="flex justify-end gap-2">{#if item.status === 'RECEIVED' || item.status === 'STAGED' || item.status === 'FAILED'}<Button size="sm" variant="outline" disabled={enqueueMutation.isPending} onclick={() => enqueue(item.id)}>Enqueue</Button>{/if}{#if item.status === 'AWAITING_REVIEW' || item.status === 'READY'}<Button href={`/rework?item=${encodeURIComponent(item.id)}`} size="sm" variant="outline">Review</Button>{/if}{#if item.status !== 'COMMITTED' && item.status !== 'REJECTED'}<Button size="sm" variant="ghost" disabled={discardMutation.isPending} onclick={() => discard(item.id)}>Discard</Button>{/if}</div></TableCell>
							</TableRow>
						{/each}
					</TableBody>
				</Table>
			</div>
		{/if}
	{/if}
</div>

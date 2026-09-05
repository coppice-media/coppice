<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { page } from '$app/state';
	import { toast } from 'svelte-sonner';
	import {
		createTable,
		rowSelectionFeature,
		tableFeatures,
		type ColumnDef
	} from '@tanstack/svelte-table';
	import { Alert, AlertDescription, AlertTitle } from '$lib/components/ui/alert';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '$lib/components/ui/empty';
	import { Input } from '$lib/components/ui/input';
	import { Textarea } from '$lib/components/ui/textarea';
	import { Separator } from '$lib/components/ui/separator';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '$lib/components/ui/table';
	import StatusBadge from '$lib/components/StatusBadge.svelte';
	import ProviderSearchDialog from '$lib/components/ProviderSearchDialog.svelte';
	import { request } from '$lib/graphql/client';
	import {
		BulkApplyIngestMetadataDocument,
		IngestBulkItemsDocument,
		type IngestDropItemFieldsFragment,
		type IngestMetadataFieldSelectionInput
	} from '$lib/graphql/generated/graphql';
	import { formatBytes } from '$lib/ingest/helpers';

	type BulkRow = IngestDropItemFieldsFragment;
	type Failure = { dropItemId: string; message: string };
	const features = tableFeatures({ rowSelectionFeature });
	const columns: ColumnDef<typeof features, BulkRow>[] = [
		{ id: 'filename', accessorKey: 'filename', header: 'Source' },
		{ id: 'status', accessorKey: 'status', header: 'Status' },
		{ id: 'score', accessorKey: 'qualityReport.score', header: 'Score' }
	];
	let rows = $state<BulkRow[]>([]);
	const table = createTable({
		features,
		columns,
		get data() {
			return rows;
		},
		getRowId: (row) => row.id
	});

	const queryClient = useQueryClient();
	let idInput = $state('');
	let selectedIds = $state<string[]>([]);
	let title = $state('');
	let summary = $state('');
	let publisher = $state('');
	let year = $state('');
	let genres = $state('');
	let failures = $state<Failure[]>([]);
	let searchOpen = $state(false);
	let searchItemId = $state<string | null>(null);

	const bulkQuery = createQuery(() => ({
		queryKey: ['bulk-items', selectedIds],
		queryFn: () => request(IngestBulkItemsDocument, { ids: selectedIds }),
		enabled: browser && selectedIds.length > 0
	}));
	const applyMutation = createMutation(() => ({
		mutationFn: (selections: IngestMetadataFieldSelectionInput[]) =>
			request(BulkApplyIngestMetadataDocument, {
				input: { dropItemIds: table.getSelectedRowIds(), selections, strategy: 'FILL_GAPS' }
			}),
		onSuccess: (result) => {
				failures = result.bulkApplyIngestMetadata.failures;
				void queryClient.invalidateQueries({ queryKey: ['bulk-items'] });
				void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
				if (failures.length) toast.warning(`${failures.length} item${failures.length === 1 ? '' : 's'} could not be updated.`);
				else toast.success('Recipe applied to the selected items.');
			},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to apply the bulk recipe.')
	}));

	let selectedCount = $derived(table.getSelectedRowIds().length);
	let loadedRows = $derived(bulkQuery.data?.ingestBulkItems ?? []);
	let canApply = $derived(selectedCount > 0 && Boolean(title.trim() || summary.trim() || publisher.trim() || year.trim() || genres.trim()));

	$effect(() => {
		if (bulkQuery.data) rows = [...bulkQuery.data.ingestBulkItems];
	});
	$effect(() => {
		const fromUrl = page.url.searchParams.get('ids');
		if (fromUrl && !selectedIds.length) selectedIds = parseIds(fromUrl);
	});

	function parseIds(value: string): string[] {
		return [...new Set(value.split(/[\s,]+/).map((id) => id.trim()).filter(Boolean))];
	}

	function loadIds(): void {
		const ids = parseIds(idInput);
		if (!ids.length) return;
		selectedIds = [...new Set([...selectedIds, ...ids])];
		idInput = '';
		table.resetRowSelection();
	}

	function removeId(id: string): void {
		selectedIds = selectedIds.filter((current) => current !== id);
		table.resetRowSelection();
	}
	function openProviderSearch(id: string): void {
		searchItemId = id;
		searchOpen = true;
	}

	function parseValue(raw: string, field: string): unknown {
		if (field === 'YEAR') return Number(raw);
		if (field === 'GENRES') {
			try {
				return JSON.parse(raw);
			} catch {
				return raw.split(',').map((entry) => entry.trim()).filter(Boolean);
			}
		}
		return raw;
	}

	function recipe(): IngestMetadataFieldSelectionInput[] {
		return [
			['TITLE', title],
			['SUMMARY', summary],
			['PUBLISHER', publisher],
			['YEAR', year],
			['GENRES', genres]
		]
			.filter(([, value]) => value.trim())
			.map(([field, value]) => ({ field: field as IngestMetadataFieldSelectionInput['field'], mode: 'MANUAL', value: parseValue(value, field) }));
	}

	function applyRecipe(): void {
		if (canApply) applyMutation.mutate(recipe());
	}
</script>

<svelte:head><title>Bulk metadata · Stump ingest</title></svelte:head>

<div class="flex flex-col gap-6">
	<div>
		<p class="text-sm font-medium text-primary">Field-level apply</p>
		<h1 class="text-3xl font-semibold tracking-tight">Bulk metadata editor</h1>
		<p class="mt-1 max-w-3xl text-muted-foreground">Load staged item IDs, select rows, and apply one validated recipe. Each item keeps its own revision and reports bounded failures.</p>
	</div>
	<Card>
		<CardHeader><CardTitle>Load staged items</CardTitle><CardDescription>Paste drop-item IDs separated by spaces, commas, or new lines.</CardDescription></CardHeader>
		<CardContent class="flex flex-col gap-3"><div class="flex flex-col gap-2 sm:flex-row"><Input class="flex-1" aria-label="Drop item IDs" bind:value={idInput} onkeydown={(event) => { if (event.key === 'Enter') loadIds(); }} /><Button onclick={loadIds}>Load items</Button></div>{#if selectedIds.length}<div class="flex flex-wrap gap-2">{#each selectedIds as id (id)}<Badge variant="secondary" class="gap-2">{id.slice(0, 18)}<button type="button" aria-label={`Remove ${id}`} class="text-muted-foreground hover:text-foreground" onclick={() => removeId(id)}>×</button></Badge>{/each}</div>{/if}</CardContent>
	</Card>

	{#if !selectedIds.length}
		<Empty><EmptyHeader><EmptyTitle>No items loaded</EmptyTitle><EmptyDescription>Enter one or more staged item IDs above to begin a bulk edit.</EmptyDescription></EmptyHeader></Empty>
	{:else if bulkQuery.isPending}
		<div class="flex flex-col gap-3">{#each Array(5) as _, index (index)}<Skeleton class="h-14 w-full" />{/each}</div>
	{:else if bulkQuery.isError}
		<Alert variant="destructive"><AlertTitle>Unable to load items</AlertTitle><AlertDescription>{bulkQuery.error instanceof Error ? bulkQuery.error.message : 'The server did not return bulk items.'}</AlertDescription></Alert>
	{:else if !loadedRows.length}
		<Empty><EmptyHeader><EmptyTitle>No matching items</EmptyTitle><EmptyDescription>Check the IDs and ensure the items are visible to your account.</EmptyDescription></EmptyHeader></Empty>
	{:else}
		<Card>
			<CardHeader><CardTitle>Selection</CardTitle><CardDescription>{selectedCount} of {loadedRows.length} loaded item{loadedRows.length === 1 ? '' : 's'} selected</CardDescription></CardHeader>
			<CardContent class="p-0"><div class="overflow-x-auto"><Table><TableHeader><TableRow><TableHead class="w-12"><input type="checkbox" checked={table.getIsAllRowsSelected()} aria-label="Select all loaded items" onclick={() => table.toggleAllRowsSelected(!table.getIsAllRowsSelected())} /></TableHead><TableHead>Source</TableHead><TableHead>Size</TableHead><TableHead>Status</TableHead><TableHead>Quality</TableHead></TableRow></TableHeader><TableBody>{#each table.getRowModel().rows as row (row.id)}<TableRow data-state={row.getIsSelected() ? 'selected' : undefined}><TableCell><input type="checkbox" checked={row.getIsSelected()} aria-label={`Select ${row.original.filename}`} onclick={row.getToggleSelectedHandler()} /></TableCell><TableCell class="max-w-[28rem]"><div class="truncate font-medium">{row.original.filename}</div><div class="truncate text-xs text-muted-foreground">{row.original.id}</div></TableCell><TableCell class="whitespace-nowrap text-muted-foreground">{formatBytes(row.original.sizeBytes)}</TableCell><TableCell><StatusBadge status={row.original.status} /></TableCell><TableCell>{row.original.qualityReport?.score ?? '—'}</TableCell></TableRow>{/each}</TableBody></Table></div></CardContent>
		</Card>
		<div class="grid gap-6 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
			<Card><CardHeader><CardTitle>Recipe</CardTitle><CardDescription>Only non-empty fields are sent as MANUAL picks; other metadata stays unchanged.</CardDescription></CardHeader><CardContent class="grid gap-4"><label class="grid gap-2 text-sm font-medium">Title<Input bind:value={title} /></label><label class="grid gap-2 text-sm font-medium">Summary<Textarea rows={3} bind:value={summary} /></label><label class="grid gap-2 text-sm font-medium">Publisher<Input bind:value={publisher} /></label><label class="grid gap-2 text-sm font-medium">Year<Input type="number" bind:value={year} /></label><label class="grid gap-2 text-sm font-medium">Genres<Textarea rows={2} bind:value={genres} /></label><Button disabled={!canApply || applyMutation.isPending} onclick={applyRecipe}>{applyMutation.isPending ? 'Applying…' : `Apply recipe to ${selectedCount} selected`}</Button></CardContent></Card>
			<Card><CardHeader><CardTitle>Failures</CardTitle><CardDescription>Each failure is isolated to one item; successful items remain applied.</CardDescription></CardHeader><CardContent>{#if failures.length}<div class="flex flex-col gap-3">{#each failures as failure (failure.dropItemId)}<div class="rounded-lg border border-destructive/30 bg-destructive/5 p-3"><p class="font-medium">{failure.dropItemId}</p><p class="text-sm text-muted-foreground">{failure.message}</p></div>{/each}</div>{:else}<Empty><EmptyHeader><EmptyTitle>No failures</EmptyTitle><EmptyDescription>Bulk apply results will appear here when a recipe is submitted.</EmptyDescription></EmptyHeader></Empty>{/if}</CardContent></Card>
			<CardContent class="p-0"><div class="overflow-x-auto"><Table><TableHeader><TableRow><TableHead class="w-12"><input type="checkbox" checked={table.getIsAllRowsSelected()} aria-label="Select all loaded items" onclick={() => table.toggleAllRowsSelected(!table.getIsAllRowsSelected())} /></TableHead><TableHead>Source</TableHead><TableHead>Size</TableHead><TableHead>Status</TableHead><TableHead>Quality</TableHead><TableHead class="text-right">Metadata</TableHead></TableRow></TableHeader><TableBody>{#each table.getRowModel().rows as row (row.id)}<TableRow data-state={row.getIsSelected() ? 'selected' : undefined}><TableCell><input type="checkbox" checked={row.getIsSelected()} aria-label={`Select ${row.original.filename}`} onclick={row.getToggleSelectedHandler()} /></TableCell><TableCell class="max-w-[28rem]"><div class="truncate font-medium">{row.original.filename}</div><div class="truncate text-xs text-muted-foreground">{row.original.id}</div></TableCell><TableCell class="whitespace-nowrap text-muted-foreground">{formatBytes(row.original.sizeBytes)}</TableCell><TableCell><StatusBadge status={row.original.status} /></TableCell><TableCell>{row.original.qualityReport?.score ?? '—'}</TableCell><TableCell class="text-right"><Button size="sm" variant="outline" onclick={() => openProviderSearch(row.original.id)}>Find metadata</Button></TableCell></TableRow>{/each}</TableBody></Table></div></CardContent>
		</div>
	{/if}
	<Separator />
	<ProviderSearchDialog
		bind:open={searchOpen}
		dropItemId={searchItemId}
		onApplied={() => {
			void queryClient.invalidateQueries({ queryKey: ['bulk-items'] });
		}}
	/>
	<p class="text-sm text-muted-foreground">This table uses TanStack Table v9 row IDs, so selection remains tied to immutable drop-item IDs rather than row positions.</p>
</div>

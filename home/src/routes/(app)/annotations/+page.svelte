<script lang="ts">
	/**
	 * `/annotations` — the annotation hub.
	 *
	 * One list of every highlight, note, and bookmark the user has, across
	 * every book and every source: the native `media_annotations`/`bookmarks`
	 * rows this console's reader writes, and the liseur-sync CAS records
	 * pushed by their devices (NickelCoppice on a Kobo, coppice.koplugin,
	 * Liseur).
	 *
	 * Filters and the page number live in the URL so a filtered hub can be
	 * linked and the back button works.
	 */
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import UploadIcon from '@lucide/svelte/icons/upload';
	import XIcon from '@lucide/svelte/icons/x';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConsoleAnnotationBooksDocument,
		ConsoleAnnotationsDocument,
		ConsoleDeleteAnnotationDocument,
		ConsoleRunAnnotationSyncDocument,
		ConsoleUpdateAnnotationDocument,
		type AnnotationKind,
		type DeviceKind
	} from '$lib/graphql/generated/graphql';
	import { countNoun } from '$lib/format';
	import {
		NO_FACETS,
		annotationFilter,
		groupByBook,
		type AnnotationFacets,
		type AnnotationLane
	} from '$lib/annotations';
	import AnnotationBookGroup from '$lib/components/annotations/AnnotationBookGroup.svelte';
	import AnnotationFilters from '$lib/components/annotations/AnnotationFilters.svelte';

	const PAGE_SIZE = 50;

	const params = $derived(page.url.searchParams);
	const pageNumber = $derived(Math.max(1, Number(params.get('page') ?? '1') || 1));
	const facets = $derived<AnnotationFacets>({
		...NO_FACETS,
		kind: (params.get('kind') as AnnotationKind | null) ?? null,
		lane: (params.get('lane') as AnnotationLane | null) ?? null,
		source: (params.get('source') as DeviceKind | null) ?? null,
		sourceDeviceId: params.get('device'),
		book: params.get('book'),
		sinceDays: params.get('since'),
		search: params.get('search')
	});

	const queryClient = useQueryClient();

	const annotationsQuery = createQuery(() => ({
		queryKey: ['annotations', facets, pageNumber],
		queryFn: () =>
			request(ConsoleAnnotationsDocument, {
				filter: annotationFilter(facets),
				pagination: { page: pageNumber, pageSize: PAGE_SIZE }
			}),
		enabled: browser
	}));
	const result = $derived(annotationsQuery.data?.annotations);
	const groups = $derived(groupByBook(result?.items ?? []));
	const focusedDeviceName = $derived(
		facets.sourceDeviceId
			? (result?.items.find(
					(item) =>
						item.sourceDeviceId === facets.sourceDeviceId && item.sourceDeviceName
				)?.sourceDeviceName ?? facets.sourceDeviceId)
			: null
	);

	// The book selector follows the same applied filters, so a device deep link
	// never offers books that belong only to another annotation source.
	const booksQuery = createQuery(() => ({
		queryKey: ['annotation-books', facets],
		queryFn: () => request(ConsoleAnnotationBooksDocument, { filter: annotationFilter(facets) }),
		enabled: browser
	}));
	const books = $derived.by(() => {
		const seen = new Map<string, string>();
		for (const item of booksQuery.data?.annotations.items ?? []) {
			if (item.book.mediaId) seen.set(item.book.mediaId, item.book.title);
		}
		return [...seen]
			.map(([mediaId, title]) => ({ mediaId, title }))
			.sort((left, right) => left.title.localeCompare(right.title));
	});

	let savingId = $state<string | null>(null);
	let deletingId = $state<string | null>(null);

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['annotations'] });
		void queryClient.invalidateQueries({ queryKey: ['annotation-books'] });
		// The reader draws the same rows as overlays.
		void queryClient.invalidateQueries({ queryKey: ['readerAnnotations'] });
	}

	const saveNote = createMutation(() => ({
		mutationFn: (variables: { id: string; annotationText: string | null }) =>
			request(ConsoleUpdateAnnotationDocument, { input: variables }),
		onSuccess: () => {
			invalidate();
			toast.success('Note saved.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'The note could not be saved.'),
		onSettled: () => (savingId = null)
	}));

	const removeAnnotation = createMutation(() => ({
		mutationFn: (id: string) => request(ConsoleDeleteAnnotationDocument, { id }),
		onSuccess: () => {
			invalidate();
			toast.success('Annotation deleted.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'The annotation could not be deleted.'),
		onSettled: () => (deletingId = null)
	}));

	const runSync = createMutation(() => ({
		mutationFn: () => request(ConsoleRunAnnotationSyncDocument, {}),
		onSuccess: (data) => {
			const enabled = data.runAnnotationSync.sinks.filter((sink) => sink.enabled);
			if (enabled.length === 0) {
				toast.info('No export sink is enabled yet — configure one in sink settings.');
				return;
			}
			toast.success(`Export queued for ${countNoun(enabled.length, 'sink')}.`);
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'The export could not be queued.')
	}));

	function navigate(patch: Record<string, string | null>, resetPage = true): void {
		const url = new URL(page.url);
		for (const [key, value] of Object.entries(patch)) {
			if (value === null || value === '') url.searchParams.delete(key);
			else url.searchParams.set(key, value);
		}
		if (resetPage) url.searchParams.delete('page');
		void goto(url, { replaceState: true, noScroll: true, keepFocus: true });
	}

	function applyFacets(patch: Partial<AnnotationFacets>): void {
		navigate({
			...('kind' in patch ? { kind: patch.kind ?? null } : {}),
			...('lane' in patch ? { lane: patch.lane ?? null } : {}),
			...('source' in patch ? { source: patch.source ?? null } : {}),
			...('sourceDeviceId' in patch ? { device: patch.sourceDeviceId ?? null } : {}),
			...('book' in patch ? { book: patch.book ?? null } : {}),
			...('sinceDays' in patch ? { since: patch.sinceDays ?? null } : {}),
			...('search' in patch ? { search: patch.search ?? null } : {})
		});
	}

	function save(id: string, annotationText: string | null): void {
		savingId = id;
		saveNote.mutate({ id, annotationText });
	}

	function remove(id: string): void {
		deletingId = id;
		removeAnnotation.mutate(id);
	}
</script>

<svelte:head>
	<title>Annotations · Coppice</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div class="flex flex-wrap items-start gap-3">
		<div class="mr-auto">
			<h1 class="text-2xl font-semibold tracking-tight">
				{focusedDeviceName ? `${focusedDeviceName} annotations` : 'Annotations'}
			</h1>
			<p class="mt-1 max-w-2xl text-sm text-muted-foreground">
				{focusedDeviceName
					? `Annotations synced from ${focusedDeviceName}. This view is filtered to that registered device; account-wide history remains available by clearing the filter.`
					: 'Every highlight, note, and bookmark you have, grouped by book — whether you made it in this reader or on a device that syncs back to this server.'}
			</p>
		</div>
		{#if focusedDeviceName}
			<Button size="sm" variant="outline" href={resolve('/annotations')}>
				<XIcon aria-hidden="true" />
				Clear device filter
			</Button>
		{/if}
		<Button
			size="sm"
			variant="outline"
			disabled={runSync.isPending}
			onclick={() => runSync.mutate()}
		>
			<UploadIcon aria-hidden="true" />
			{runSync.isPending ? 'Queueing…' : 'Export now'}
		</Button>
		<Button size="sm" variant="ghost" href={`${resolve('/connections')}#exports`}>
			<SettingsIcon aria-hidden="true" />
			Export settings
		</Button>
	</div>

	<AnnotationFilters
		{facets}
		{books}
		deviceLabel={(deviceId) =>
			focusedDeviceName && facets.sourceDeviceId === deviceId ? focusedDeviceName : deviceId}
		onfacets={applyFacets}
	/>

	{#if annotationsQuery.isPending}
		<div class="flex flex-col gap-4">
			{#each { length: 3 } as _, index (index)}
				<Skeleton class="h-44 w-full rounded-xl" />
			{/each}
		</div>
	{:else if annotationsQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load annotations</AlertTitle>
			<AlertDescription>
				{annotationsQuery.error instanceof Error
					? annotationsQuery.error.message
					: 'The server did not return your annotations.'}
			</AlertDescription>
		</Alert>
	{:else if groups.length === 0}
		<Empty class="rounded-xl border border-dashed bg-card">
			<EmptyHeader>
				<EmptyTitle>Nothing here yet</EmptyTitle>
				<EmptyDescription>
					{#if result?.total === 0 && params.size === 0}
						Highlight a passage while reading, or sync a device that carries annotations, and it
						shows up here.
					{:else}
						No annotation matches these filters.
					{/if}
				</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else}
		<div class="flex flex-wrap items-center gap-3 text-sm text-muted-foreground">
			<span>
				{countNoun(result?.total ?? 0, 'annotation')} across
				{countNoun(result?.bookCount ?? 0, 'book')}
			</span>
			{#if pageNumber > 1 || result?.hasNext}
				<div class="ml-auto flex items-center gap-2">
					<Button
						size="sm"
						variant="outline"
						aria-label="Previous page"
						disabled={pageNumber <= 1}
						onclick={() => navigate({ page: String(pageNumber - 1) }, false)}
					>
						<ChevronLeftIcon />
					</Button>
					<span class="tabular-nums">Page {pageNumber}</span>
					<Button
						size="sm"
						variant="outline"
						aria-label="Next page"
						disabled={!result?.hasNext}
						onclick={() => navigate({ page: String(pageNumber + 1) }, false)}
					>
						<ChevronRightIcon />
					</Button>
				</div>
			{/if}
		</div>

		{#each groups as group (group.book.key)}
			<AnnotationBookGroup {group} {savingId} {deletingId} onsave={save} ondelete={remove} />
		{/each}

		{#if runSync.data?.runAnnotationSync.pending}
			<Badge variant="secondary" class="self-start">Export pending</Badge>
		{/if}
	{/if}
</div>

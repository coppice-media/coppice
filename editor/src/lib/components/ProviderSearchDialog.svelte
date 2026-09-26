<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Cover } from '@stump/ui/components/ui/cover';
	import * as Dialog from '@stump/ui/components/ui/dialog';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import * as Select from '@stump/ui/components/ui/select';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { request } from '@stump/ui/graphql/client';
	import {
		IngestProviderCatalogDocument,
		IngestProviderSearchDocument,
		LookupIngestCandidateDocument,
		type IngestMediaKind,
		type IngestProviderSearchQuery
	} from '$lib/graphql/generated/graphql';

	type SearchHit = IngestProviderSearchQuery['ingestProviderSearch'][number];

	interface Props {
		open?: boolean;
		/** Drop item the resulting candidate is stored against. */
		dropItemId?: string | null;
		/** Called after a hit was stored as a candidate; callers invalidate their own queries here. */
		onApplied?: () => void;
	}

	let {
		open = $bindable(false),
		dropItemId = null,
		onApplied
	}: Props = $props();

	const MEDIA_KINDS: Array<{ value: IngestMediaKind; label: string }> = [
		{ value: 'COMIC_ARCHIVE', label: 'Comic archive' },
		{ value: 'COMIC_RAR_ARCHIVE', label: 'Comic RAR archive' },
		{ value: 'EPUB', label: 'EPUB' },
		{ value: 'PDF', label: 'PDF' }
	];

	const queryClient = useQueryClient();
	let queryText = $state('');
	let mediaKind = $state<'ANY' | IngestMediaKind>('ANY');
	let selectedProviders = $state<string[]>([]);

	const catalogQuery = createQuery(() => ({
		queryKey: ['ingest-provider-catalog', true],
		queryFn: () => request(IngestProviderCatalogDocument, { includeDisabled: false }),
		enabled: browser && open
	}));

	// Chips default to providers that advertise SEARCH; fall back to every
	// configured provider so the dialog stays usable with older registries.
	let chipProviders = $derived.by(() => {
		const providers = catalogQuery.data?.ingestProviderCatalog ?? [];
		const capable = providers.filter(
			(provider) => provider.configured && provider.capabilities.includes('SEARCH')
		);
		return capable.length ? capable : providers.filter((provider) => provider.configured);
	});

	const searchMutation = createMutation(() => ({
		mutationFn: () =>
			request(IngestProviderSearchDocument, {
				query: queryText.trim(),
				mediaKind: mediaKind === 'ANY' ? undefined : mediaKind,
				providers: selectedProviders.length ? selectedProviders : undefined,
				limit: 10
			}),
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Provider search failed.')
	}));

	const lookupMutation = createMutation(() => ({
		mutationFn: (hit: SearchHit) =>
			request(LookupIngestCandidateDocument, {
				dropItemId: dropItemId as string,
				providerId: hit.providerId,
				externalId: hit.externalId
			}),
		onSuccess: () => {
			if (dropItemId) void queryClient.invalidateQueries({ queryKey: ['ingest-item', dropItemId] });
			onApplied?.();
			toast.success('Provider hit stored as a candidate for review.');
			open = false;
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to store the candidate.')
	}));

	let hits = $derived(searchMutation.data?.ingestProviderSearch ?? []);
	let canSearch = $derived(Boolean(queryText.trim()) && Boolean(dropItemId) && !searchMutation.isPending);

	function runSearch(): void {
		if (!queryText.trim() || !dropItemId) return;
		searchMutation.mutate();
	}

	function toggleProvider(id: string): void {
		selectedProviders = selectedProviders.includes(id)
			? selectedProviders.filter((current) => current !== id)
			: [...selectedProviders, id];
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>Search providers</Dialog.Title>
			<Dialog.Description>
				Search external metadata providers and store a hit as a candidate on this item.
			</Dialog.Description>
		</Dialog.Header>

		<div class="flex flex-col gap-4 overflow-y-auto pr-1">
			<div class="flex flex-col gap-2 sm:flex-row">
				<Input
					class="flex-1"
					placeholder="Title, series, or filename"
					aria-label="Search query"
					bind:value={queryText}
					onkeydown={(event) => {
						if (event.key === 'Enter') runSearch();
					}}
				/>
				<Select.Root type="single" bind:value={mediaKind}>
					<Select.Trigger class="w-full sm:w-44" aria-label="Media kind">
						{mediaKind === 'ANY' ? 'Any media kind' : MEDIA_KINDS.find((kind) => kind.value === mediaKind)?.label}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value="ANY" label="Any media kind" />
						{#each MEDIA_KINDS as kind (kind.value)}
							<Select.Item value={kind.value} label={kind.label} />
						{/each}
					</Select.Content>
				</Select.Root>
				<Button disabled={!canSearch} onclick={runSearch}>
					{searchMutation.isPending ? 'Searching…' : 'Search'}
				</Button>
			</div>

			{#if chipProviders.length}
				<div class="flex flex-wrap items-center gap-2">
					<span class="text-xs text-muted-foreground">
						{selectedProviders.length ? 'Restricting to' : 'All providers'}:
					</span>
					{#each chipProviders as provider (provider.id)}
						<button type="button" onclick={() => toggleProvider(provider.id)}>
							<Badge
								variant={selectedProviders.includes(provider.id) ? 'default' : 'outline'}
								class="cursor-pointer"
							>
								{provider.name}
							</Badge>
						</button>
					{/each}
				</div>
			{/if}

			{#if searchMutation.isPending}
				<div class="flex flex-col gap-3">
					{#each Array(3) as _, index (index)}
						<div class="flex gap-3">
							<Skeleton class="h-16 w-12 rounded" />
							<div class="flex flex-1 flex-col gap-2 py-1">
								<Skeleton class="h-4 w-2/3" />
								<Skeleton class="h-3 w-1/3" />
							</div>
						</div>
					{/each}
				</div>
			{:else if searchMutation.isError}
				<Alert variant="destructive">
					<AlertTitle>Search failed</AlertTitle>
					<AlertDescription>
						{searchMutation.error instanceof Error
							? searchMutation.error.message
							: 'The server could not complete the provider search.'}
					</AlertDescription>
				</Alert>
			{:else if searchMutation.isSuccess && !hits.length}
				<Empty>
					<EmptyHeader>
						<EmptyTitle>No hits</EmptyTitle>
						<EmptyDescription>No provider returned a match for this query. Try a shorter title or fewer provider filters.</EmptyDescription>
					</EmptyHeader>
				</Empty>
			{:else if hits.length}
				<ul class="flex flex-col gap-3" aria-label="Provider search hits">
					{#each hits as hit (hit.providerId + ':' + hit.externalId)}
						<li class="flex gap-3 rounded-lg border p-3">
							<Cover src={hit.coverUrl} class="h-16 w-12 rounded border" />
							<div class="flex min-w-0 flex-1 flex-col gap-1">
								<div class="flex flex-wrap items-center gap-2">
									<span class="truncate font-medium">{hit.title}</span>
									{#if hit.year}<span class="text-xs tabular-nums text-muted-foreground">{hit.year}</span>{/if}
									<Badge variant="secondary">{hit.providerId}</Badge>
									<span class="text-xs tabular-nums text-muted-foreground">score {hit.score.toFixed(2)}</span>
								</div>
								{#if hit.summary}
									<p class="line-clamp-2 text-sm text-muted-foreground">{hit.summary}</p>
								{/if}
								<div class="flex justify-end">
									<Button
										size="sm"
										variant="outline"
										disabled={lookupMutation.isPending || !dropItemId}
										onclick={() => lookupMutation.mutate(hit)}
									>
										{lookupMutation.isPending ? 'Storing…' : 'Use as candidate'}
									</Button>
								</div>
							</div>
						</li>
					{/each}
				</ul>
			{:else}
				<Empty>
					<EmptyHeader>
						<EmptyTitle>Search metadata providers</EmptyTitle>
						<EmptyDescription>Enter a query above to find matching records. A chosen hit is fetched in full and stored as a candidate alongside the analysis results.</EmptyDescription>
					</EmptyHeader>
				</Empty>
			{/if}
		</div>

		<Dialog.Footer>
			<Button variant="outline" onclick={() => (open = false)}>Close</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

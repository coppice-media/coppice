<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createQuery } from '@tanstack/svelte-query';
	import SearchIcon from '@lucide/svelte/icons/search';
	import XIcon from '@lucide/svelte/icons/x';
	import { Button } from '@stump/ui/components/ui/button';
	import { Input } from '@stump/ui/components/ui/input';
	import { request } from '@stump/ui/graphql/client';
	import { BookSearchDocument, ConsoleLibrarySeriesDocument } from '$lib/graphql/generated/graphql';

	const RESULT_LIMIT = 5;
	let value = $state('');
	let query = $state('');
	let focused = $state(false);
	let timer: ReturnType<typeof setTimeout> | null = null;

	const booksQuery = createQuery(() => ({
		queryKey: ['shell-search', 'books', query],
		queryFn: () => request(BookSearchDocument, { query, limit: RESULT_LIMIT }),
		enabled: browser && query.length >= 2
	}));
	const seriesQuery = createQuery(() => ({
		queryKey: ['shell-search', 'series', query],
		queryFn: () =>
			request(ConsoleLibrarySeriesDocument, {
				filter: { name: { contains: query } },
				orderBy: [{ series: { field: 'NAME', direction: 'ASC' } }],
				pagination: { offset: { page: 1, pageSize: RESULT_LIMIT } }
			}),
		enabled: browser && query.length >= 2
	}));

	const books = $derived(booksQuery.data?.searchBooks ?? []);
	const series = $derived(seriesQuery.data?.series.nodes ?? []);
	const searching = $derived(query.length >= 2 && (booksQuery.isPending || seriesQuery.isPending));
	const hasResults = $derived(books.length > 0 || series.length > 0);
	const showResults = $derived(focused && value.trim().length >= 2);

	function scheduleSearch(): void {
		if (timer) clearTimeout(timer);
		const next = value.trim();
		if (next.length < 2) {
			query = '';
			return;
		}
		timer = setTimeout(() => {
			query = next;
			timer = null;
		}, 220);
	}

	function clear(): void {
		value = '';
		query = '';
		focused = false;
		if (timer) {
			clearTimeout(timer);
			timer = null;
		}
	}

	function closeOnEscape(event: KeyboardEvent): void {
		if (event.key === 'Escape') {
			clear();
			(event.currentTarget as HTMLInputElement).blur();
		}
	}
</script>

<div class="relative min-w-0 flex-1 max-w-xl lg:mx-auto lg:px-6">
	<label class="sr-only" for="home-local-search">Search visible books and series</label>
	<SearchIcon
		aria-hidden="true"
		class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
	/>
	<Input
		id="home-local-search"
		value={value}
		placeholder="Search visible books and series"
		autocomplete="off"
		class="h-9 bg-muted/35 pr-9 pl-9 text-sm"
		aria-controls="home-local-search-results"
		aria-expanded={showResults}
		role="combobox"
		onfocus={() => (focused = true)}
		oninput={(event) => {
			value = event.currentTarget.value;
			scheduleSearch();
		}}
		onkeydown={closeOnEscape}
	/>
	{#if value}
		<Button
			variant="ghost"
			size="icon-xs"
			class="absolute top-1/2 right-1 -translate-y-1/2"
			aria-label="Clear search"
			onclick={clear}
		>
			<XIcon aria-hidden="true" />
		</Button>
	{/if}

	{#if showResults}
		<div
			id="home-local-search-results"
			role="listbox"
			tabindex="-1"
			class="absolute top-[calc(100%+0.5rem)] right-0 left-0 z-50 max-h-[min(26rem,calc(100vh-5rem))] overflow-y-auto rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl"
			onmouseleave={() => (focused = false)}
		>
			{#if searching}
				<p class="px-3 py-4 text-sm text-muted-foreground" role="status">Searching your visible library…</p>
			{:else if !hasResults && !booksQuery.isError && !seriesQuery.isError}
				<p class="px-3 py-4 text-sm text-muted-foreground">No visible books or series match “{query}”.</p>
			{:else}
				{#if series.length}
					<p class="px-3 pt-2 pb-1 text-[0.68rem] font-semibold tracking-[0.14em] text-muted-foreground uppercase">
						Series
					</p>
					{#each series as entry (entry.id)}
						<a
							role="option"
							aria-selected="false"
							class="block rounded-md px-3 py-2 text-sm hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:outline-none"
							href={resolve('/(app)/series/[id]', { id: entry.id })}
							onclick={() => (focused = false)}
						>
							<span class="block truncate font-medium">{entry.resolvedName}</span>
							<span class="block truncate text-xs text-muted-foreground">{entry.mediaCount} books</span>
						</a>
					{/each}
				{/if}
				{#if books.length}
					<p class="px-3 pt-2 pb-1 text-[0.68rem] font-semibold tracking-[0.14em] text-muted-foreground uppercase">
						Books
					</p>
					{#each books as entry (entry.mediaId)}
						<a
							role="option"
							aria-selected="false"
							class="block rounded-md px-3 py-2 text-sm hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent focus-visible:outline-none"
							href={resolve('/(app)/book/[mediaId]', { mediaId: entry.mediaId })}
							onclick={() => (focused = false)}
						>
							<span class="block truncate font-medium">{entry.title}</span>
							<span class="block truncate text-xs text-muted-foreground">{entry.authors.join(', ') || entry.kind.toLowerCase()}</span>
						</a>
					{/each}
				{/if}
			{/if}
		</div>
	{/if}
</div>

<style>
	@media (max-width: 639px) {
		:global(#home-local-search) {
			font-size: 0.8125rem;
		}
	}
</style>

<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import SearchIcon from '@lucide/svelte/icons/search';
	import XIcon from '@lucide/svelte/icons/x';
	import { Button } from '@stump/ui/components/ui/button';
	import * as InputGroup from '@stump/ui/components/ui/input-group';
	import { PageHeader } from '@stump/ui/components/ui/page-header';
	import UnifiedSearchResults from '$lib/components/UnifiedSearchResults.svelte';
	import { LIBRARY_MIN_CHARS, SearchTerm } from '$lib/search.svelte';

	const initialQuery = page.url.searchParams.get('q') ?? '';
	const routeQuery = $derived(page.url.searchParams.get('q') ?? '');
	const term = new SearchTerm(initialQuery);
	let appliedRouteQuery = $state(initialQuery);

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		const query = term.flush();
		if (!query) return;
		void goto(`${resolve('/(app)/search')}?q=${encodeURIComponent(query)}`, { replaceState: true });
	}

	// Back/forward and header searches land here with a new `?q`; apply it once.
	$effect(() => {
		if (routeQuery === appliedRouteQuery) return;
		appliedRouteQuery = routeQuery;
		term.reset(routeQuery);
	});
</script>

<svelte:head>
	<title>{term.query ? `Search “${term.query}”` : 'Search'} · Coppice</title>
	<meta name="description" content="Search your visible library and request books from Hardcover or Audible." />
</svelte:head>

<div class="flex flex-col gap-6">
	<PageHeader
		title="Search"
		description="Search books in your library and request new titles from Hardcover or Audible."
	/>

	<form class="flex max-w-2xl gap-2" onsubmit={submit}>
		<InputGroup.Root class="flex-1">
			<InputGroup.Addon>
				<SearchIcon aria-hidden="true" />
			</InputGroup.Addon>
			<InputGroup.Input
				id="unified-search-query"
				type="search"
				value={term.value}
				placeholder="Search your library and Hardcover"
				autocomplete="off"
				aria-label="Search your library and Hardcover"
				oninput={(event) => term.update(event.currentTarget.value)}
			/>
			{#if term.value}
				<InputGroup.Addon align="inline-end">
					<InputGroup.Button size="icon-xs" aria-label="Clear search" onclick={() => term.reset()}>
						<XIcon aria-hidden="true" />
					</InputGroup.Button>
				</InputGroup.Addon>
			{/if}
		</InputGroup.Root>
		<Button type="submit" variant="outline" disabled={term.value.trim().length < LIBRARY_MIN_CHARS}>
			Search
		</Button>
	</form>

	<UnifiedSearchResults query={term.query} typing={term.typing} />
</div>

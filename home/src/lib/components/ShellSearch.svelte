<script lang="ts">
	/**
	 * The header search: a field-shaped trigger (⌘K / Ctrl+K) that opens a
	 * command palette. Library, Hardcover and Audible hits are three
	 * independent queries, so the library group renders the moment it answers
	 * and the Audible group waits only for Hardcover, which it is deduped against.
	 */
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import HeadphonesIcon from '@lucide/svelte/icons/headphones';
	import SearchIcon from '@lucide/svelte/icons/search';
	import XIcon from '@lucide/svelte/icons/x';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import * as Command from '@stump/ui/components/ui/command';
	import { Kbd, KbdGroup } from '@stump/ui/components/ui/kbd';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import ExternalHitActions from '$lib/components/requests/ExternalHitActions.svelte';
	import { safeCoverUrl } from '$lib/requests';
	import {
		audibleHitMeta,
		createAudibleSearch,
		createExternalSearch,
		createLibrarySearch,
		EXTERNAL_MIN_CHARS,
		externalHitHref,
		externalHitMeta,
		LIBRARY_MIN_CHARS,
		onlyOnAudible,
		SearchTerm,
		type ExternalHit
	} from '$lib/search.svelte';

	/** Hits per group in the palette; the rest live on `/search`. */
	const LIMIT = 5;
	/** Audible rows are the overflow of the overflow: three is plenty here. */
	const AUDIBLE_LIMIT = 3;
	const modifierKey = browser && /Mac|iPhone|iPad/.test(navigator.userAgent) ? '⌘' : 'Ctrl';

	let open = $state(false);
	const term = new SearchTerm();
	const libraryQuery = createLibrarySearch(() => term.query, LIMIT);
	const externalQuery = createExternalSearch(() => term.query, LIMIT);
	const audibleQuery = createAudibleSearch(() => term.query, LIMIT);
	const library = $derived(libraryQuery.data?.librarySearch ?? []);
	const external = $derived(externalQuery.data?.externalBookSearch ?? null);
	const typed = $derived(term.value.trim());
	const audible = $derived(
		onlyOnAudible(audibleQuery.data?.audibleBookSearch.hits ?? [], external?.hits ?? [], AUDIBLE_LIMIT)
	);
	// The dedupe needs Hardcover's answer, so the Audible group also waits for it.
	const audiblePending = $derived(term.typing || audibleQuery.isPending || externalQuery.isPending);
	// The group is silent unless it has something Hardcover lacks: no error text, no empty note.
	const audibleVisible = $derived(
		typed.length >= EXTERNAL_MIN_CHARS &&
			(audiblePending || (!audibleQuery.isError && !audibleQuery.data?.audibleBookSearch.error && audible.length > 0))
	);

	function handleWindowKeydown(event: KeyboardEvent): void {
		if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === 'k') {
			event.preventDefault();
			open = !open;
		}
	}

	function navigate(href: string): void {
		open = false;
		void goto(href);
	}

	function viewAll(): void {
		const query = term.flush();
		if (query) navigate(`${resolve('/(app)/search')}?q=${encodeURIComponent(query)}`);
	}
</script>

<svelte:window onkeydown={handleWindowKeydown} />

<Button
	variant="outline"
	class="h-9 min-w-0 flex-1 justify-start bg-muted/35 px-3 font-normal text-muted-foreground lg:mx-auto lg:max-w-xl"
	aria-keyshortcuts="Control+K Meta+K"
	onclick={() => (open = true)}
>
	<SearchIcon data-icon="inline-start" aria-hidden="true" />
	<span class="truncate">Search library and Hardcover…</span>
	<KbdGroup class="ml-auto hidden sm:inline-flex" aria-hidden="true">
		<Kbd>{modifierKey}</Kbd>
		<Kbd>K</Kbd>
	</KbdGroup>
</Button>

{#snippet thumb(src: string | undefined, square = false)}
	{#if src}
		<img {src} alt="" class={['h-12 shrink-0 rounded-sm border object-cover', square ? 'w-12' : 'w-8']} loading="lazy" />
	{:else}
		<span
			class={['flex h-12 shrink-0 items-center justify-center rounded-sm border bg-muted/40', square ? 'w-12' : 'w-8']}
			aria-hidden="true"
		>
			<BookOpenIcon class="size-3.5 text-muted-foreground" />
		</span>
	{/if}
{/snippet}

{#snippet externalRow(hit: ExternalHit, meta: string, audible: boolean)}
	<Command.Item
		value="external:{hit.provider}:{hit.remoteId}"
		onSelect={() => navigate(externalHitHref(hit))}
		class="gap-3 [&>svg:last-child]:hidden"
	>
		{@render thumb(safeCoverUrl(hit.coverUrl), audible)}
		<span class="min-w-0 flex-1">
			<span class="block truncate">{hit.title}</span>
			<span class="block truncate text-xs text-muted-foreground">
				{hit.authors || 'Author not provided'}{meta ? ` · ${meta}` : ''}
			</span>
		</span>
		{#if audible}
			<Badge variant="outline" class="hidden sm:inline-flex" aria-label="Audiobook">
				<HeadphonesIcon aria-hidden="true" />
				Audible
			</Badge>
		{/if}
		<!-- Svelte delegates these handlers, so stopping here keeps the click
		     and Enter/Space with the control instead of selecting the item. -->
		<div
			role="presentation"
			class="shrink-0"
			onclick={(event) => event.stopPropagation()}
			onkeydown={(event) => {
				if (event.key === 'Enter' || event.key === ' ') event.stopPropagation();
			}}
		>
			<ExternalHitActions {hit} onnavigate={() => (open = false)} />
		</div>
	</Command.Item>
{/snippet}

{#snippet note(text: string)}
	<p class="px-2 py-1.5 text-sm text-muted-foreground">{text}</p>
{/snippet}

{#snippet skeletonRows(count: number, square = false)}
	<Command.Loading aria-label="Searching…">
		{#each { length: count } as _, index (index)}
			<div class="flex items-center gap-3 px-2 py-1.5">
				<Skeleton class={['h-12 shrink-0 rounded-sm', square ? 'w-12' : 'w-8']} />
				<div class="flex flex-1 flex-col gap-1.5">
					<Skeleton class="h-3.5 w-1/2" />
					<Skeleton class="h-3 w-1/3" />
				</div>
			</div>
		{/each}
	</Command.Loading>
{/snippet}

<Command.Dialog
	bind:open
	shouldFilter={false}
	loop
	title="Search"
	description="Search your library and request books from Hardcover or Audible."
	class="sm:max-w-xl"
>
	<div class="relative">
		<Command.Input
			placeholder="Search your library and Hardcover"
			autocomplete="off"
			class="pr-7"
			bind:value={() => term.value, (next) => term.update(next)}
		/>
		{#if term.value}
			<Button
				variant="ghost"
				size="icon-xs"
				class="absolute top-1/2 right-2.5 -translate-y-1/2"
				aria-label="Clear search"
				onclick={() => term.reset()}
			>
				<XIcon aria-hidden="true" />
			</Button>
		{/if}
	</div>
	<!-- The dialog sits a third of the way down, so the list stops short of the
	     viewport's bottom and the footer stays pinned inside its scroll. -->
	<Command.List class="max-h-[min(28rem,calc(66dvh-6rem))]">
		{#if typed.length < LIBRARY_MIN_CHARS}
			<p class="py-6 text-center text-sm text-muted-foreground">Type at least two characters to search.</p>
		{:else}
			<Command.Group heading="In your library">
				{#if term.typing || libraryQuery.isPending}
					{@render skeletonRows(3)}
				{:else if libraryQuery.isError}
					{@render note(libraryQuery.error instanceof Error ? libraryQuery.error.message : 'Library search unavailable')}
				{:else if library.length === 0}
					{@render note('No library matches')}
				{:else}
					{#each library as hit (hit.mediaId)}
						<Command.LinkItem
							value="library:{hit.mediaId}"
							href={resolve('/(app)/book/[mediaId]', { mediaId: hit.mediaId })}
							onSelect={() => (open = false)}
							class="gap-3"
						>
							{@render thumb(hit.thumbnailUrl)}
							<span class="min-w-0 flex-1">
								<span class="block truncate">{hit.title}</span>
								<span class="block truncate text-xs text-muted-foreground">
									{hit.authors || 'Author not provided'}{hit.seriesName ? ` · ${hit.seriesName}` : ''}
								</span>
							</span>
							<Badge variant="outline">{hit.isAudiobook ? 'Audiobook' : hit.extension.toUpperCase()}</Badge>
						</Command.LinkItem>
					{/each}
				{/if}
			</Command.Group>
			<Command.Separator forceMount />
			<Command.Group heading="Request from Hardcover">
				{#if typed.length < EXTERNAL_MIN_CHARS}
					{@render note('Keep typing to search Hardcover')}
				{:else if term.typing || externalQuery.isPending}
					{@render skeletonRows(2)}
				{:else if externalQuery.isError || external?.error}
					{@render note('Hardcover search unavailable')}
				{:else if !external?.hits.length}
					{@render note('No Hardcover matches')}
				{:else}
					{#each external.hits as hit (`${hit.provider}:${hit.remoteId}`)}
						{@render externalRow(hit, externalHitMeta(hit), false)}
					{/each}
				{/if}
			</Command.Group>
			{#if audibleVisible}
				<Command.Separator forceMount />
				<Command.Group heading="Only on Audible">
					{#if audiblePending}
						{@render skeletonRows(1, true)}
					{:else}
						{#each audible as hit (`${hit.provider}:${hit.remoteId}`)}
							{@render externalRow(hit, audibleHitMeta(hit), true)}
						{/each}
					{/if}
				</Command.Group>
			{/if}
			<Command.Group class="sticky bottom-0 border-t bg-popover">
				<Command.Item value="view-all" onSelect={viewAll} class="gap-3 [&>svg:last-child]:hidden">
					<SearchIcon aria-hidden="true" />
					<span class="truncate">View all results for “{typed}”</span>
				</Command.Item>
			</Command.Group>
		{/if}
	</Command.List>
</Command.Dialog>

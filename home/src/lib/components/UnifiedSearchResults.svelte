<script lang="ts">
	/**
	 * The `/search` results: independent sections, each with its own
	 * skeleton, so library matches show the moment they land instead of
	 * waiting for Hardcover. The Audible section lists only the works
	 * Hardcover did not return, so it waits for Hardcover before it shows.
	 */
	import { resolve } from '$app/paths';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent } from '@stump/ui/components/ui/card';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import ExternalBookRequestCard from '$lib/components/ExternalBookRequestCard.svelte';
	import {
		createAudibleSearch,
		createExternalSearch,
		createLibrarySearch,
		EXTERNAL_MIN_CHARS,
		LIBRARY_MIN_CHARS,
		onlyOnAudible
	} from '$lib/search.svelte';

	/** Audible rows that survive the Hardcover dedupe; the rest are noise. */
	const AUDIBLE_LIMIT = 5;

	let {
		query,
		typing = false
	}: {
		/** The settled search text; empty below `LIBRARY_MIN_CHARS`. */
		query: string;
		/** The field has moved past `query` and the debounce is still running. */
		typing?: boolean;
	} = $props();

	const libraryQuery = createLibrarySearch(() => query, 20);
	const externalQuery = createExternalSearch(() => query, 10);
	const audibleQuery = createAudibleSearch(() => query, AUDIBLE_LIMIT);
	const library = $derived(libraryQuery.data?.librarySearch ?? []);
	const external = $derived(externalQuery.data?.externalBookSearch ?? null);
	const audible = $derived(
		onlyOnAudible(audibleQuery.data?.audibleBookSearch.hits ?? [], external?.hits ?? [], AUDIBLE_LIMIT)
	);
	const audiblePending = $derived(typing || audibleQuery.isPending || externalQuery.isPending);
	// Silent on error or when Hardcover already covers everything: no note, no heading.
	const audibleVisible = $derived(
		(typing || query.length >= EXTERNAL_MIN_CHARS) &&
			(audiblePending || (!audibleQuery.isError && !audibleQuery.data?.audibleBookSearch.error && audible.length > 0))
	);
	const libraryError = $derived(
		libraryQuery.error instanceof Error ? libraryQuery.error.message : 'The search service is unavailable.'
	);
</script>

{#snippet note(text: string)}
	<p class="text-sm text-muted-foreground">{text}</p>
{/snippet}

{#if !typing && query.length < LIBRARY_MIN_CHARS}
	{@render note('Enter at least two characters to search.')}
{:else}
	<div class="flex flex-col gap-8">
		<section class="flex flex-col gap-3" aria-labelledby="library-results">
			<h2 id="library-results" class="text-sm font-semibold tracking-wide">In your library</h2>
			{#if typing || libraryQuery.isPending}
				<div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
					{#each { length: 3 } as _, index (index)}
						<Skeleton class="h-24 rounded-xl" />
					{/each}
				</div>
			{:else if libraryQuery.isError}
				{@render note(libraryError)}
			{:else if library.length === 0}
				{@render note(`Nothing in your visible library matches “${query}”.`)}
			{:else}
				<div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
					{#each library as hit (hit.mediaId)}
						<a
							href={resolve('/(app)/book/[mediaId]', { mediaId: hit.mediaId })}
							class="block min-w-0 rounded-xl focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
						>
							<Card size="sm" class="h-full transition-colors hover:ring-primary/50">
								<CardContent class="flex-row gap-3">
									{#if hit.thumbnailUrl}
										<img src={hit.thumbnailUrl} alt="" class="h-18 w-12 shrink-0 rounded-md border object-cover" loading="lazy" />
									{:else}
										<div class="flex h-18 w-12 shrink-0 items-center justify-center rounded-md border bg-muted/40" aria-hidden="true">
											<BookOpenIcon class="size-4 text-muted-foreground" />
										</div>
									{/if}
									<div class="flex min-w-0 flex-1 flex-col gap-1.5">
										<div class="min-w-0">
											<p class="truncate font-medium" title={hit.title}>{hit.title}</p>
											<p class="truncate text-xs text-muted-foreground">
												{hit.authors || 'Author not provided'}{hit.seriesName ? ` · ${hit.seriesName}` : ''}
											</p>
										</div>
										<Badge variant="outline">{hit.isAudiobook ? 'Audiobook' : hit.extension.toUpperCase()}</Badge>
									</div>
								</CardContent>
							</Card>
						</a>
					{/each}
				</div>
			{/if}
		</section>

		<section class="flex flex-col gap-3" aria-labelledby="hardcover-results">
			<h2 id="hardcover-results" class="text-sm font-semibold tracking-wide">Request from Hardcover</h2>
			{#if !typing && query.length < EXTERNAL_MIN_CHARS}
				{@render note('Enter at least three characters to search Hardcover.')}
			{:else if typing || externalQuery.isPending}
				<div class="grid gap-3 md:grid-cols-2">
					{#each { length: 2 } as _, index (index)}
						<Skeleton class="h-32 rounded-xl" />
					{/each}
				</div>
			{:else if externalQuery.isError || external?.error}
				{@render note('Hardcover search unavailable')}
			{:else if !external?.hits.length}
				{@render note(`No ${external?.provider ?? 'Hardcover'} books match “${query}”.`)}
			{:else}
				<div class="grid gap-3 md:grid-cols-2">
					{#each external.hits as hit (`${hit.provider}:${hit.remoteId}`)}
						<ExternalBookRequestCard {hit} />
					{/each}
				</div>
			{/if}
		</section>

		{#if audibleVisible}
			<section class="flex flex-col gap-3" aria-labelledby="audible-results">
				<h2 id="audible-results" class="text-sm font-semibold tracking-wide">Only on Audible</h2>
				{#if audiblePending}
					<div class="grid gap-3 md:grid-cols-2">
						<Skeleton class="h-32 rounded-xl" />
					</div>
				{:else}
					<div class="grid gap-3 md:grid-cols-2">
						{#each audible as hit (`${hit.provider}:${hit.remoteId}`)}
							<ExternalBookRequestCard {hit} />
						{/each}
					</div>
				{/if}
			</section>
		{/if}
	</div>
{/if}

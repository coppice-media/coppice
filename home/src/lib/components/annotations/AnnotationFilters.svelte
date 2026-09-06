<script lang="ts">
	/**
	 * The hub's filter bar. Every facet lives in the page's query string, so
	 * this component only reports patches and renders what it is given.
	 */
	import XIcon from '@lucide/svelte/icons/x';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import * as Select from '@stump/ui/components/ui/select';
	import type { AnnotationKind, DeviceKind } from '$lib/graphql/generated/graphql';
	import {
		ANY_OPTION,
		KIND_LABELS,
		KIND_OPTIONS,
		SINCE_OPTIONS,
		SOURCE_LABELS,
		SOURCE_OPTIONS,
		activeFacets,
		type AnnotationFacets
	} from '$lib/annotations';

	let {
		facets,
		books = [],
		onfacets
	}: {
		facets: AnnotationFacets;
		/** Every book the user has annotated, for the book selector. */
		books?: { mediaId: string; title: string }[];
		onfacets: (patch: Partial<AnnotationFacets>) => void;
	} = $props();

	const chips = $derived(
		activeFacets(
			facets,
			(mediaId) => books.find((book) => book.mediaId === mediaId)?.title ?? mediaId
		)
	);
	const bookLabel = $derived(
		facets.book
			? (books.find((book) => book.mediaId === facets.book)?.title ?? facets.book)
			: books.length
				? 'Any book'
				: 'No books yet'
	);
	const sinceLabel = $derived(
		SINCE_OPTIONS.find((option) => option.value === facets.sinceDays)?.label ?? 'Any time'
	);

	// The draft mirrors the applied search without submitting on every keystroke.
	let search = $state('');
	$effect(() => {
		search = facets.search ?? '';
	});

	function submitSearch(event: SubmitEvent): void {
		event.preventDefault();
		onfacets({ search: search.trim() ? search.trim() : null });
	}
</script>

<div class="flex flex-col gap-3">
	<div class="flex flex-wrap items-end gap-3">
		<form class="flex items-end gap-2" onsubmit={submitSearch}>
			<div class="flex flex-col gap-1">
				<Label class="text-xs text-muted-foreground" for="annotation-search">Text contains</Label>
				<Input
					id="annotation-search"
					class="w-52"
					bind:value={search}
					placeholder="spice must flow"
				/>
			</div>
			<Button size="sm" variant="outline" type="submit">Search</Button>
		</form>

		<div class="flex flex-col gap-1">
			<Label class="text-xs text-muted-foreground" for="annotation-kind">Kind</Label>
			<Select.Root
				type="single"
				value={facets.kind ?? ANY_OPTION}
				onValueChange={(value) =>
					onfacets({ kind: value === ANY_OPTION ? null : (value as AnnotationKind) })}
			>
				<Select.Trigger id="annotation-kind" class="w-36">
					{facets.kind ? KIND_LABELS[facets.kind] : 'Any kind'}
				</Select.Trigger>
				<Select.Content>
					<Select.Item value={ANY_OPTION} label="Any kind" />
					{#each KIND_OPTIONS as option (option.value)}
						<Select.Item value={option.value} label={option.label} />
					{/each}
				</Select.Content>
			</Select.Root>
		</div>

		<div class="flex flex-col gap-1">
			<Label class="text-xs text-muted-foreground" for="annotation-source">Source</Label>
			<Select.Root
				type="single"
				value={facets.source ?? ANY_OPTION}
				onValueChange={(value) =>
					onfacets({ source: value === ANY_OPTION ? null : (value as DeviceKind) })}
			>
				<Select.Trigger id="annotation-source" class="w-44">
					{facets.source ? SOURCE_LABELS[facets.source] : 'Any source'}
				</Select.Trigger>
				<Select.Content>
					<Select.Item value={ANY_OPTION} label="Any source" />
					{#each SOURCE_OPTIONS as option (option.value)}
						<Select.Item value={option.value} label={option.label} />
					{/each}
				</Select.Content>
			</Select.Root>
		</div>

		<div class="flex flex-col gap-1">
			<Label class="text-xs text-muted-foreground" for="annotation-book">Book</Label>
			<Select.Root
				type="single"
				value={facets.book ?? ANY_OPTION}
				onValueChange={(value) => onfacets({ book: value === ANY_OPTION ? null : value })}
				disabled={books.length === 0}
			>
				<Select.Trigger id="annotation-book" class="w-52">{bookLabel}</Select.Trigger>
				<Select.Content>
					<Select.Item value={ANY_OPTION} label="Any book" />
					{#each books as book (book.mediaId)}
						<Select.Item value={book.mediaId} label={book.title} />
					{/each}
				</Select.Content>
			</Select.Root>
		</div>

		<div class="flex flex-col gap-1">
			<Label class="text-xs text-muted-foreground" for="annotation-since">Since</Label>
			<Select.Root
				type="single"
				value={facets.sinceDays ?? ANY_OPTION}
				onValueChange={(value) => onfacets({ sinceDays: value === ANY_OPTION ? null : value })}
			>
				<Select.Trigger id="annotation-since" class="w-40">{sinceLabel}</Select.Trigger>
				<Select.Content>
					<Select.Item value={ANY_OPTION} label="Any time" />
					{#each SINCE_OPTIONS as option (option.value)}
						<Select.Item value={option.value} label={option.label} />
					{/each}
				</Select.Content>
			</Select.Root>
		</div>
	</div>

	{#if chips.length}
		<div class="flex flex-wrap items-center gap-2">
			{#each chips as chip (chip.key)}
				<Badge variant="secondary" class="h-7 gap-1 pr-1">
					{chip.label}
					<Button
						size="icon-xs"
						variant="ghost"
						aria-label={`Clear ${chip.key} filter`}
						onclick={() => {
							if (chip.key === 'search') search = '';
							onfacets({ [chip.key]: null });
						}}
					>
						<XIcon />
					</Button>
				</Badge>
			{/each}
		</div>
	{/if}
</div>

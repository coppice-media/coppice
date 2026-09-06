<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import XIcon from '@lucide/svelte/icons/x';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import * as Select from '@stump/ui/components/ui/select';
	import { request } from '@stump/ui/graphql/client';
	import { ConsoleTagsDocument, type ReadingStatus } from '$lib/graphql/generated/graphql';
	import {
		ANY_OPTION,
		BOOK_FORMATS,
		READING_STATUS_LABELS,
		READING_STATUS_OPTIONS,
		activeFacets,
		type BookFacets
	} from '$lib/library';

	let {
		facets,
		onfacets
	}: {
		facets: BookFacets;
		onfacets: (patch: Partial<BookFacets>) => void;
	} = $props();

	const tagsQuery = createQuery(() => ({
		queryKey: ['tags'],
		queryFn: () => request(ConsoleTagsDocument, {}),
		enabled: browser
	}));
	const tags = $derived(tagsQuery.data?.tags ?? []);
	const chips = $derived(activeFacets(facets));

	// The draft mirrors the applied facet, which the page owns (it lives in the
	// URL), without submitting on every keystroke.
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
				<Label class="text-xs text-muted-foreground" for="book-search">Title contains</Label>
				<Input id="book-search" class="w-48" bind:value={search} placeholder="Saga" />
			</div>
			<Button size="sm" variant="outline" type="submit">Search</Button>
		</form>

		<div class="flex flex-col gap-1">
			<Label class="text-xs text-muted-foreground" for="filter-status">Read status</Label>
			<Select.Root
				type="single"
				value={facets.status ?? ANY_OPTION}
				onValueChange={(value) =>
					onfacets({ status: value === ANY_OPTION ? null : (value as ReadingStatus) })}
			>
				<Select.Trigger id="filter-status" class="w-40">
					{facets.status ? READING_STATUS_LABELS[facets.status] : 'Any status'}
				</Select.Trigger>
				<Select.Content>
					<Select.Item value={ANY_OPTION} label="Any status" />
					{#each READING_STATUS_OPTIONS as option (option.value)}
						<Select.Item value={option.value} label={option.label} />
					{/each}
				</Select.Content>
			</Select.Root>
		</div>

		<div class="flex flex-col gap-1">
			<Label class="text-xs text-muted-foreground" for="filter-format">Format</Label>
			<Select.Root
				type="single"
				value={facets.format ?? ANY_OPTION}
				onValueChange={(value) => onfacets({ format: value === ANY_OPTION ? null : value })}
			>
				<Select.Trigger id="filter-format" class="w-32">
					{facets.format ? facets.format.toUpperCase() : 'Any format'}
				</Select.Trigger>
				<Select.Content>
					<Select.Item value={ANY_OPTION} label="Any format" />
					{#each BOOK_FORMATS as format (format)}
						<Select.Item value={format} label={format.toUpperCase()} />
					{/each}
				</Select.Content>
			</Select.Root>
		</div>

		<div class="flex flex-col gap-1">
			<Label class="text-xs text-muted-foreground" for="filter-tag">Tag</Label>
			<Select.Root
				type="single"
				value={facets.tag ?? ANY_OPTION}
				onValueChange={(value) => onfacets({ tag: value === ANY_OPTION ? null : value })}
				disabled={tags.length === 0}
			>
				<Select.Trigger id="filter-tag" class="w-40">
					{facets.tag ?? (tags.length ? 'Any tag' : 'No tags yet')}
				</Select.Trigger>
				<Select.Content>
					<Select.Item value={ANY_OPTION} label="Any tag" />
					{#each tags as tag (tag.id)}
						<Select.Item value={tag.name} label={tag.name} />
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

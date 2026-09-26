<script lang="ts">
	/**
	 * One pill for a request's format and narrator: a book toggle for an
	 * ebook, a headphones toggle for an audiobook, and a chevron on the
	 * headphones' edge that opens the narrator picker. Both toggles on is
	 * `ANY`; the last one on cannot be released, so a request always names at
	 * least one format. The narrator is a preference for `AUDIOBOOK`/`ANY`
	 * only: it biases release ranking, never filters.
	 *
	 * The same pill serves the search palette, a `/search` card, the request
	 * form and (with `lockFormat`) an existing request.
	 */
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import BookIcon from '@lucide/svelte/icons/book';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ChevronUpIcon from '@lucide/svelte/icons/chevron-up';
	import HeadphonesIcon from '@lucide/svelte/icons/headphones';
	import { Badge } from '@stump/ui/components/ui/badge';
	import * as Popover from '@stump/ui/components/ui/popover';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import * as ToggleGroup from '@stump/ui/components/ui/toggle-group';
	import * as Tooltip from '@stump/ui/components/ui/tooltip';
	import { request } from '@stump/ui/graphql/client';
	import { cn } from '@stump/ui/utils.js';
	import { AudiobookNarratorsDocument, type RequestFormat } from '$lib/graphql/generated/graphql';
	import {
		audioLengthLabel,
		formatIncludesAudio,
		type FormatAvailability,
		type NarratorLookup
	} from '$lib/requests';

	type NarratorOption = {
		name: string;
		sources: readonly string[];
		durationSeconds?: number | null;
		abridged?: boolean | null;
	};

	let {
		value = $bindable('ANY'),
		narrator = $bindable(null),
		availability = {},
		lookup = null,
		audiobookOnly = false,
		lockFormat = false,
		disabled = false,
		label = 'Request format',
		onnarratorchange
	}: {
		value?: RequestFormat;
		/** The preferred narrator; `null` is “any narrator”. */
		narrator?: string | null;
		availability?: FormatAvailability;
		/** Identifies the work for `audiobookNarrators`; without it the picker only offers “Any narrator”. */
		lookup?: NarratorLookup | null;
		/** An Audible edition: no book toggle, the format is `AUDIOBOOK`. */
		audiobookOnly?: boolean;
		/** Show the format without letting it change, as on an existing request. */
		lockFormat?: boolean;
		disabled?: boolean;
		/** Names the group for screen readers, e.g. `Request format for <title>`. */
		label?: string;
		/** Fired after the user picks a narrator, with `null` for “any”. */
		onnarratorchange?: (narrator: string | null) => void;
	} = $props();

	const pressed = $derived(value === 'ANY' ? ['EBOOK', 'AUDIOBOOK'] : [value]);
	const audioSelected = $derived(formatIncludesAudio(value));
	const audioLength = $derived(audioLengthLabel(availability.audioSeconds));
	// An Audible row already states its length, so only the (changeable) narrator repeats beside the pill.
	const lengthNote = $derived(audiobookOnly ? null : audioLength);
	/** The format is shown, not chosen: an existing request or an Audible edition. */
	const formatLocked = $derived(lockFormat || audiobookOnly);

	function select(next: string[]): void {
		if (next.length === 0) return;
		value = next.length === 2 ? 'ANY' : (next[0] as RequestFormat);
		// An ebook-only request has no narrator to prefer.
		if (value === 'EBOOK') narrator = null;
	}

	let open = $state(false);
	const narratorsQuery = createQuery(() => ({
		queryKey: ['audiobook-narrators', lookup?.provider, lookup?.remoteId, lookup?.title, lookup?.authors ?? null],
		queryFn: () =>
			request(AudiobookNarratorsDocument, {
				provider: lookup?.provider ?? '',
				remoteId: lookup?.remoteId ?? '',
				title: lookup?.title ?? '',
				authors: lookup?.authors ?? null
			}),
		enabled: browser && open && lookup !== null,
		staleTime: 60 * 60_000
	}));
	const options = $derived.by((): NarratorOption[] => {
		const found: NarratorOption[] = narratorsQuery.data?.audiobookNarrators ?? [];
		// A narrator chosen earlier (or seeded by an Audible hit) stays pickable even
		// when the lookup does not list it.
		if (narrator && !found.some((option) => option.name === narrator)) {
			return [{ name: narrator, sources: [] }, ...found];
		}
		return found;
	});

	function choose(name: string | null): void {
		narrator = name;
		open = false;
		onnarratorchange?.(name);
	}
</script>

{#snippet formatItem(kind: RequestFormat, kindLabel: string, Icon: typeof BookIcon, known: boolean | null | undefined, tooltip: string)}
	<Tooltip.Root>
		<Tooltip.Trigger>
			{#snippet child({ props })}
				<ToggleGroup.Item
					{...props}
					value={kind}
					aria-label={kindLabel}
					class={cn(
						'text-muted-foreground data-[state=on]:bg-primary data-[state=on]:text-primary-foreground data-[state=on]:hover:bg-primary/90 data-[state=off]:hover:text-foreground',
						known === false && 'opacity-45 data-[state=on]:opacity-75',
						formatLocked && 'disabled:opacity-100'
					)}
				>
					<!-- `!`: a selected command row recolours every svg; the pressed icon must stay legible on the primary fill. -->
					<Icon
						aria-hidden="true"
						class="opacity-60 group-data-[state=on]/toggle:text-primary-foreground! group-data-[state=on]/toggle:opacity-100"
					/>
				</ToggleGroup.Item>
			{/snippet}
		</Tooltip.Trigger>
		<Tooltip.Content>{tooltip}</Tooltip.Content>
	</Tooltip.Root>
{/snippet}

{#snippet narratorOption(name: string | null, primary: string, option: NarratorOption | null)}
	{@const selected = name === narrator}
	<button
		type="button"
		role="option"
		aria-selected={selected}
		class="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm outline-none hover:bg-muted focus-visible:bg-muted"
		onclick={() => choose(name)}
	>
		<CheckIcon aria-hidden="true" class={cn('size-4 shrink-0', !selected && 'opacity-0')} />
		<span class="flex min-w-0 flex-1 flex-col">
			<span class="truncate">{primary}</span>
			{#if option}
				{@const length = audioLengthLabel(option.durationSeconds)}
				{#if length || option.abridged || option.sources.length}
					<span class="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
						{#if length}<span class="tabular-nums">{length}</span>{/if}
						{#if option.abridged}
							<Badge variant="outline" class="h-4 px-1.5 text-[10px]">Abridged</Badge>
						{/if}
						{#if option.sources.length}
							<span class="capitalize">{option.sources.join(' · ')}</span>
						{/if}
					</span>
				{/if}
			{/if}
		</span>
	</button>
{/snippet}

<div class="flex flex-wrap items-center gap-x-2 gap-y-1">
	<ToggleGroup.Root
		type="multiple"
		bind:value={() => pressed, select}
		size="sm"
		variant="outline"
		aria-label={label}
		disabled={disabled || formatLocked}
	>
		{#if !audiobookOnly}
			{@render formatItem(
				'EBOOK',
				'Ebook',
				BookIcon,
				availability.hasEbook,
				availability.hasEbook === false ? 'No ebook edition known on Hardcover' : 'Ebook'
			)}
		{/if}
		{@render formatItem(
			'AUDIOBOOK',
			'Audiobook',
			HeadphonesIcon,
			availability.hasAudiobook,
			availability.hasAudiobook === false
				? 'No audiobook edition known on Hardcover'
				: audioLength
					? `Audiobook · ${audioLength}`
					: 'Audiobook'
		)}
		<Popover.Root bind:open>
			<Popover.Trigger disabled={disabled || !audioSelected}>
				{#snippet child({ props })}
					<button
						{...props}
						type="button"
						aria-label="Preferred narrator"
						class="inline-flex h-8 w-5 shrink-0 items-center justify-center rounded-r-md border border-l-0 border-input bg-transparent text-muted-foreground shadow-xs transition-[color,box-shadow] outline-none hover:bg-muted hover:text-foreground focus-visible:z-10 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 data-[state=open]:bg-muted data-[state=open]:text-foreground [&_svg]:size-3.5"
					>
						<ChevronUpIcon aria-hidden="true" />
					</button>
				{/snippet}
			</Popover.Trigger>
			<Popover.Content side="top" align="end" sideOffset={6} class="w-72 gap-0 p-1">
				<p class="px-2 py-1.5 text-xs font-medium text-muted-foreground">Preferred narrator</p>
				<div role="listbox" aria-label="Preferred narrator" class="flex max-h-64 flex-col overflow-y-auto">
					{@render narratorOption(null, 'Any narrator', null)}
					{#if lookup === null}
						<!-- Nothing to look up yet: the form has no provider reference. -->
					{:else if narratorsQuery.isPending}
						{#each { length: 3 } as _, index (index)}
							<div class="flex items-center gap-2 px-2 py-1.5" aria-hidden="true">
								<span class="size-4 shrink-0"></span>
								<div class="flex flex-1 flex-col gap-1.5">
									<Skeleton class="h-3.5 w-2/3" />
									<Skeleton class="h-3 w-1/3" />
								</div>
							</div>
						{/each}
					{:else if narratorsQuery.isError}
						<p class="px-2 py-1.5 text-xs text-muted-foreground">Narrator lookup unavailable — any narrator will do</p>
					{:else if options.length === 0}
						<p class="px-2 py-1.5 text-xs text-muted-foreground">No narrators found — any narrator will do</p>
					{:else}
						{#each options as option (option.name)}
							{@render narratorOption(option.name, option.name, option)}
						{/each}
					{/if}
				</div>
			</Popover.Content>
		</Popover.Root>
	</ToggleGroup.Root>
	{#if audioSelected && (narrator || lengthNote)}
		<span class="text-xs text-muted-foreground">
			{#if lengthNote}<span class="tabular-nums">· {lengthNote}</span>{/if}
			{#if narrator}<span>· Narr. {narrator}</span>{/if}
		</span>
	{/if}
</div>

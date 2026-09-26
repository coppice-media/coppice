<script lang="ts">
	import { resolve } from '$app/paths'
	import { browser } from '$app/environment'
	import AppleIcon from '@lucide/svelte/icons/apple'
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left'
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right'
	import LayoutGridIcon from '@lucide/svelte/icons/layout-grid'
	import MonitorIcon from '@lucide/svelte/icons/monitor'
	import SearchIcon from '@lucide/svelte/icons/search'
	import SmartphoneIcon from '@lucide/svelte/icons/smartphone'
	import TabletIcon from '@lucide/svelte/icons/tablet'
	import WorkflowIcon from '@lucide/svelte/icons/workflow'
	import XIcon from '@lucide/svelte/icons/x'
	import BookAudioIcon from '@lucide/svelte/icons/book-audio'
	import BookOpenTextIcon from '@lucide/svelte/icons/book-open-text'
	import BookMarkedIcon from '@lucide/svelte/icons/book-marked'
	import type { LucideIcon } from '@lucide/svelte'
	import { Button } from '@stump/ui/components/ui/button'
	import * as ToggleGroup from '@stump/ui/components/ui/toggle-group'
	import { Input } from '@stump/ui/components/ui/input'
	import {
		CLIENT_CATALOG,
		clientCapabilityAvailability,
		clientVariantForId,
		clientVariants,
		defaultClientVariant,
		missingPermissionsForClient,
		PERMISSION_LABELS,
		type ClientAppVariant,
		type ClientCatalogEntry,
		type ClientPlatformFilter,
		type ClientReadFilter,
		type DeviceCapabilityDescriptor
	} from '$lib/devices'
	import type { UserPermission } from '$lib/graphql/generated/graphql'
	import { cn } from '@stump/ui/utils.js'
	import ClientCard from './ClientCard.svelte'
	type PlatformChoice = { value: ClientPlatformFilter | 'all'; label: string; icon: LucideIcon }
	type ReadChoice = { value: ClientReadFilter; label: string; icon: LucideIcon }
	type SearchResult = { entry: ClientCatalogEntry; variant: ClientAppVariant }
	type HighlightPart = { text: string; match: boolean }

	const PLATFORM_FILTERS: readonly PlatformChoice[] = [
		{ value: 'all', label: 'All', icon: LayoutGridIcon },
		{ value: 'ios', label: 'iOS', icon: AppleIcon },
		{ value: 'android', label: 'Android', icon: SmartphoneIcon },
		{ value: 'ereaders', label: 'eReaders', icon: TabletIcon },
		{ value: 'desktop', label: 'Desktop', icon: MonitorIcon },
		{ value: 'automation', label: 'Automation', icon: WorkflowIcon }
	]
	const READ_FILTERS: readonly ReadChoice[] = [
		{ value: 'comics', label: 'Comics', icon: BookOpenTextIcon },
		{ value: 'ebooks', label: 'eBooks', icon: BookMarkedIcon },
		{ value: 'audiobooks', label: 'Audiobooks', icon: BookAudioIcon }
	]
	const LEGEND = [
		{ marker: '✓', label: 'Full support', class: 'border-border/70' },
		{ marker: '~', label: 'Partial', class: 'border-dashed border-muted-foreground/50' },
		{ marker: '–', label: 'Unsupported', class: 'border-destructive/40' },
		{ marker: '?', label: 'Unverified', class: 'border-dotted border-muted-foreground/50' }
	]

	let {
		entries = CLIENT_CATALOG,
		user,
		capabilities = null,
		hideDisabled = false,
		selectedId = $bindable(null),
		selectedVariantId = $bindable(null),
		onselect
	}: {
		entries?: readonly ClientCatalogEntry[]
		user: { isServerOwner: boolean; permissions: readonly UserPermission[] } | null
		capabilities?: readonly DeviceCapabilityDescriptor[] | null
		hideDisabled?: boolean
		selectedId?: string | null
		selectedVariantId?: string | null
		onselect?: (entry: ClientCatalogEntry | null, variant?: ClientAppVariant | null) => void
	} = $props()

	let search = $state('')
	let platformFilter = $state<ClientPlatformFilter | null>(null)
	let readFilters = $state<ClientReadFilter[]>([])
	let page = $state(1)
	let pageSize = $state(6)
	let searchFocused = $state(false)
	let activeResultIndex = $state(0)

	function normalized(value: string): string {
		return value.trim().toLocaleLowerCase()
	}

	function supportsRead(variant: ClientAppVariant, format: ClientReadFilter): boolean {
		return variant.mediaFormats.some((item) => item.format === format && item.status !== 'unsupported')
	}

	function isServerVariantAvailable(variant: ClientAppVariant): boolean {
		return clientCapabilityAvailability(variant.kind, capabilities).available
	}

	function availabilityForVariant(variant: ClientAppVariant | null): {
		available: boolean
		reason: string | null
	} {
		return variant
			? clientCapabilityAvailability(variant.kind, capabilities)
			: { available: false, reason: 'No client variant is configured.' }
	}

	function variantSearchText(entry: ClientCatalogEntry, variant: ClientAppVariant): string {
		return [
			entry.title,
			entry.description,
			entry.protocol,
			entry.setup,
			entry.caveat ?? '',
			variant.label,
			...variant.platforms,
			...variant.keywords,
			...variant.capabilities.map((capability) => capability.label),
			...variant.mediaFormats.map((format) => format.label)
		]
			.join(' ')
			.toLocaleLowerCase()
	}

	const allAppVariants = $derived.by(() => {
		const seen = new Set<string>()
		const result: SearchResult[] = []
		for (const entry of entries) {
			for (const variant of clientVariants(entry)) {
				if (hideDisabled && !isServerVariantAvailable(variant)) continue
				if (seen.has(variant.id)) continue
				seen.add(variant.id)
				result.push({ entry, variant })
			}
		}
		return result
	})

	const filteredEntries = $derived.by(() => {
		const needle = normalized(search)
		return entries.filter((entry) => {
			const variants = clientVariants(entry)
			return variants.some((variant) => {
				if (hideDisabled && !isServerVariantAvailable(variant)) return false
				if (platformFilter && !variant.filters.includes(platformFilter)) return false
				if (!readFilters.every((format) => supportsRead(variant, format))) return false
				if (!needle) return true
				return variantSearchText(entry, variant).includes(needle)
			})
		})
	})

	const searchResults = $derived.by(() => {
		const needle = normalized(search)
		if (!needle) return [] as SearchResult[]
		return allAppVariants
			.filter(({ entry, variant }) => variantSearchText(entry, variant).includes(needle))
			.sort((left, right) => {
				const leftExact = normalized(left.variant.label) === needle || normalized(left.variant.id) === needle
				const rightExact = normalized(right.variant.label) === needle || normalized(right.variant.id) === needle
				return Number(rightExact) - Number(leftExact)
			})
			.slice(0, 8)
	})

	const showSuggestions = $derived(searchFocused && search.trim().length > 0 && searchResults.length > 0)
	const totalPages = $derived(Math.max(1, Math.ceil(filteredEntries.length / pageSize)))
	const visibleEntries = $derived(filteredEntries.slice((page - 1) * pageSize, page * pageSize))
	const firstResult = $derived(filteredEntries.length ? (page - 1) * pageSize + 1 : 0)
	const lastResult = $derived(Math.min(page * pageSize, filteredEntries.length))

	function exactVariantForEntry(entry: ClientCatalogEntry): ClientAppVariant | null {
		const needle = normalized(search)
		if (!needle) return null
		return (
			clientVariants(entry).find(
				(variant) => normalized(variant.label) === needle || normalized(variant.id) === needle
			) ?? null
		)
	}

	function cardVariantForEntry(entry: ClientCatalogEntry): ClientAppVariant | null {
		return (
			exactVariantForEntry(entry) ??
			clientVariants(entry).find((variant) => isServerVariantAvailable(variant)) ??
			defaultClientVariant(entry)
		)
	}

	function choose(entry: ClientCatalogEntry, requestedVariant?: ClientAppVariant | null): void {
		const variant = requestedVariant ?? cardVariantForEntry(entry)
		if (!variant || !isServerVariantAvailable(variant)) return
		if (missingPermissionsForClient(entry, user, variant.kind).length > 0) return
		selectedId = entry.id
		selectedVariantId = variant.id
		onselect?.(entry, variant)
	}

	function chooseSearchResult(result: SearchResult): void {
		choose(result.entry, result.variant)
		searchFocused = false
	}

	function changeSearch(event: Event): void {
		search = (event.currentTarget as HTMLInputElement).value
		activeResultIndex = 0
		page = 1
	}

	function clearSearch(): void {
		search = ''
		activeResultIndex = 0
		page = 1
	}

	function changePlatformFilter(value: string): void {
		platformFilter = value && value !== 'all' ? (value as ClientPlatformFilter) : null
		page = 1
	}

	function changeReadFilters(value: string[]): void {
		readFilters = value as ClientReadFilter[]
		page = 1
	}

	function searchKeydown(event: KeyboardEvent): void {
		if (event.key === 'ArrowDown') {
			if (!searchResults.length) return
			event.preventDefault()
			searchFocused = true
			activeResultIndex = Math.min(activeResultIndex + 1, searchResults.length - 1)
			return
		}
		if (event.key === 'ArrowUp') {
			if (!searchResults.length) return
			event.preventDefault()
			activeResultIndex = Math.max(activeResultIndex - 1, 0)
			return
		}
		if (event.key === 'Enter' && showSuggestions) {
			event.preventDefault()
			chooseSearchResult(searchResults[activeResultIndex] ?? searchResults[0])
			return
		}
		if (event.key === 'Escape') {
			searchFocused = false
		}
	}

	function searchFocusout(event: FocusEvent): void {
		const wrapper = event.currentTarget as HTMLElement
		const related = event.relatedTarget as Node | null
		if (!related || !wrapper.contains(related)) searchFocused = false
	}

	function highlightParts(label: string): HighlightPart[] {
		const needle = normalized(search)
		if (!needle) return [{ text: label, match: false }]
		const index = label.toLocaleLowerCase().indexOf(needle)
		if (index < 0) return [{ text: label, match: false }]
		return [
			...(index ? [{ text: label.slice(0, index), match: false }] : []),
			{ text: label.slice(index, index + needle.length), match: true },
			...(index + needle.length < label.length
				? [{ text: label.slice(index + needle.length), match: false }]
				: [])
		]
	}

	function changePageSize(): void {
		if (!browser) return
		const width = window.innerWidth
		const next = width < 640 ? 2 : width < 1024 ? 4 : 6
		if (next !== pageSize) {
			pageSize = next
			page = 1
		}
	}

	$effect(() => {
		if (!browser) return
		changePageSize()
		window.addEventListener('resize', changePageSize)
		return () => window.removeEventListener('resize', changePageSize)
	})

	$effect(() => {
		if (page > totalPages) page = totalPages
		if (selectedId && !entries.some((entry) => entry.id === selectedId)) {
			selectedId = null
			selectedVariantId = null
			onselect?.(null, null)
		}
		if (selectedId && selectedVariantId) {
			const selectedEntry = entries.find((entry) => entry.id === selectedId)
			const selectedVariant = selectedEntry
				? clientVariants(selectedEntry).find((variant) => variant.id === selectedVariantId)
				: null
			if (!selectedVariant) {
				selectedVariantId = selectedEntry ? defaultClientVariant(selectedEntry)?.id ?? null : null
			} else if (hideDisabled && !isServerVariantAvailable(selectedVariant)) {
				selectedId = null
				selectedVariantId = null
				onselect?.(null, null)
			}
		}
	})
</script>

<div class="flex flex-col gap-3" aria-label="Client catalog">
	<div class="flex flex-col gap-2 sm:flex-row sm:items-center">
		<div class="relative min-w-0 flex-1" onfocusin={() => (searchFocused = true)} onfocusout={searchFocusout}>
			<SearchIcon class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
			<Input
				value={search}
				oninput={changeSearch}
				onkeydown={searchKeydown}
				placeholder="Search clients or apps"
				aria-label="Search clients, apps, platforms, or capabilities"
				role="combobox"
				aria-autocomplete="list"
				aria-expanded={showSuggestions}
				aria-controls="client-search-results"
				aria-activedescendant={showSuggestions ? `client-search-result-${activeResultIndex}` : undefined}
				class="pr-9 pl-9"
			/>
			{#if search}
				<Button
					type="button"
					variant="ghost"
					size="icon-sm"
					class="absolute top-1/2 right-1 -translate-y-1/2"
					aria-label="Clear client search"
					title="Clear search"
					onclick={clearSearch}
				>
					<XIcon />
				</Button>
			{/if}
			{#if showSuggestions}
				<div class="absolute top-full right-0 left-0 z-20 mt-1 overflow-hidden rounded-lg border bg-popover p-1 shadow-md" role="presentation">
					<ul id="client-search-results" role="listbox" aria-label="Matching client apps">
						{#each searchResults as result, index (`${result.entry.id}-${result.variant.id}`)}
							{@const resultAvailable = isServerVariantAvailable(result.variant)}
							{@const ResultIcon = result.variant.icon ?? result.entry.icon}
							<li id={`client-search-result-${index}`} role="option" aria-selected={index === activeResultIndex}>
								<button
									type="button"
									class={cn(
										'flex w-full items-start gap-2 rounded-md px-2.5 py-2 text-left text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50',
										index === activeResultIndex ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/60',
										!resultAvailable && 'cursor-not-allowed opacity-60'
									)}
									disabled={!resultAvailable}
									aria-disabled={!resultAvailable}
									onclick={() => chooseSearchResult(result)}
									onmouseenter={() => (activeResultIndex = index)}
								>
									<span class="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
										<ResultIcon class="size-3.5" aria-hidden="true" />
									</span>
									<span class="min-w-0 flex-1">
										<span class="block truncate font-medium">
											{#each highlightParts(result.variant.label) as part}
												<span class={part.match ? 'font-semibold text-primary' : undefined}>{part.text}</span>
											{/each}
										</span>
										<span class="block truncate text-xs text-muted-foreground">{result.entry.title}</span>
									</span>
								</button>
							</li>
						{/each}
					</ul>
				</div>
			{/if}
		</div>
		<p class="shrink-0 text-xs text-muted-foreground" aria-live="polite">
			{#if filteredEntries.length}
				Showing {firstResult}–{lastResult} of {filteredEntries.length}
			{:else}
				0 results
			{/if}
		</p>
	</div>

	<div class="flex flex-wrap items-center gap-x-3 gap-y-2" aria-label="Client filters">
		<ToggleGroup.Root
			type="single"
			variant="outline"
			size="sm"
			bind:value={() => platformFilter ?? 'all', changePlatformFilter}
			aria-label="Filter clients by platform"
			class="max-w-full"
		>
			{#each PLATFORM_FILTERS as option (option.value)}
				{@const Icon = option.icon}
				<ToggleGroup.Item value={option.value} aria-label={option.label} class="text-xs">
					<Icon class="size-3.5" aria-hidden="true" />
					<span class="hidden sm:inline">{option.label}</span>
				</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>

		<ToggleGroup.Root
			type="multiple"
			variant="outline"
			size="sm"
			bind:value={() => readFilters, changeReadFilters}
			aria-label="Formats the client must read"
			class="max-w-full"
		>
			{#each READ_FILTERS as option (option.value)}
				{@const Icon = option.icon}
				<ToggleGroup.Item value={option.value} aria-label={option.label} class="text-xs">
					<Icon class="size-3.5" aria-hidden="true" />
					<span class="hidden sm:inline">{option.label}</span>
				</ToggleGroup.Item>
			{/each}
		</ToggleGroup.Root>
	</div>

	{#if visibleEntries.length}
		<div class="grid items-stretch gap-2 sm:grid-cols-2 lg:grid-cols-3">
			{#each visibleEntries as entry (entry.id)}
				{@const variant =
					selectedId === entry.id ? clientVariantForId(entry, selectedVariantId) : cardVariantForEntry(entry)}
				{@const variants = clientVariants(entry)}
				{@const serverAvailability = availabilityForVariant(variant)}
				{@const missing = variant ? missingPermissionsForClient(entry, user, variant.kind) : []}
				{@const available = serverAvailability.available && missing.length === 0}
				<ClientCard
					icon={variant?.icon ?? entry.icon}
					title={entry.title}
					evidence={variant?.evidence}
					maturity={variant?.maturity}
					description={entry.description}
					meta={variant
						? [
								{ label: 'Runs on', value: variant.platforms.join(' · ') },
								{ label: 'App', value: variant.label },
								{ label: 'Connection', value: variant.protocol }
							]
						: []}
					capabilities={variant?.capabilities}
					mediaFormats={variant?.mediaFormats}
					selected={selectedId === entry.id}
					{available}
					onchoose={() => choose(entry)}
					data-client-card={entry.id}
				>
					{#snippet footer()}
						{#if variants.length > 1}
							<div class="flex min-w-0 flex-wrap gap-1" role="group" aria-label={`Apps in ${entry.title}`}>
								{#each variants as appVariant (appVariant.id)}
									{@const VariantIcon = appVariant.icon ?? entry.icon}
									{@const appAvailable = isServerVariantAvailable(appVariant)}
									<button
										type="button"
										class={cn(
											'inline-flex min-h-6 min-w-0 max-w-full items-center gap-1 rounded-md border px-1.5 py-0.5 text-[11px] font-medium transition-colors outline-none focus-visible:ring-3 focus-visible:ring-ring/50',
											variant?.id === appVariant.id
												? 'border-primary/60 bg-primary/10 text-primary'
												: 'border-border/70 bg-background text-muted-foreground hover:border-primary/40 hover:text-foreground',
											!appAvailable && 'cursor-not-allowed opacity-50'
										)}
										data-app-variant={appVariant.id}
										aria-pressed={variant?.id === appVariant.id}
										disabled={!appAvailable}
										title={!appAvailable ? 'Disabled by the server' : undefined}
										onclick={() => choose(entry, appVariant)}
									>
										<VariantIcon class="size-3 shrink-0" aria-hidden="true" />
										<span class="min-w-0 truncate">{appVariant.label}</span>
									</button>
								{/each}
							</div>
						{/if}
						{#if !available}
							<p class="text-[11px] font-medium text-destructive">
								{#if serverAvailability.reason}
									Unavailable: {serverAvailability.reason}
								{:else}
									Missing: {missing.map((permission) => PERMISSION_LABELS[permission] ?? permission).join(', ')}
								{/if}
							</p>
							{#if !serverAvailability.available}
								<a href={resolve('/components')} class="w-fit text-[11px] font-medium text-primary underline-offset-4 hover:underline">
									Review in Components
								</a>
							{/if}
						{/if}
					{/snippet}
				</ClientCard>
			{/each}
		</div>
	{:else}
		<div class="flex min-h-32 flex-col items-center justify-center rounded-xl border border-dashed bg-muted/20 px-4 py-5 text-center">
			{#if hideDisabled}
				<p class="text-sm font-medium">No enabled integrations match those filters</p>
				<p class="mt-1 max-w-sm text-xs text-muted-foreground">
					Turn off “Hide disabled integrations” to inspect unavailable setup options.
				</p>
			{:else}
				<p class="text-sm font-medium">No clients match those filters</p>
				<p class="mt-1 max-w-sm text-xs text-muted-foreground">
					Try a different search term, platform, or format filter.
				</p>
			{/if}
		</div>
	{/if}

	<div class="flex flex-wrap items-center gap-x-3 gap-y-2" aria-label="Catalog pagination">
		<p class="text-[10px] text-muted-foreground">Page {Math.min(page, totalPages)} of {totalPages}</p>
		<p class="hidden items-center gap-x-2.5 text-[10px] whitespace-nowrap text-muted-foreground sm:flex" aria-label="Capability legend">
			{#each LEGEND as item (item.marker)}
				<span class="inline-flex items-center gap-1">
					<span class={cn('inline-flex size-4 items-center justify-center rounded border', item.class)} aria-hidden="true">{item.marker}</span>
					{item.label}
				</span>
			{/each}
		</p>
		<div class="ml-auto flex items-center gap-1.5">
			<Button
				type="button"
				variant="outline"
				size="sm"
				disabled={page <= 1}
				aria-label="Previous clients"
				onclick={() => (page = Math.max(1, page - 1))}
			>
				<ChevronLeftIcon data-icon="inline-start" />
				Previous
			</Button>
			<Button
				type="button"
				variant="outline"
				size="sm"
				disabled={page >= totalPages}
				aria-label="Next clients"
				onclick={() => (page = Math.min(totalPages, page + 1))}
			>
				Next
				<ChevronRightIcon data-icon="inline-end" />
			</Button>
		</div>
	</div>
</div>

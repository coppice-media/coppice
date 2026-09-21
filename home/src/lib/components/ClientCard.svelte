<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge'
	import {
		clientVariants,
		type ClientAppVariant,
		type ClientCatalogEntry,
		defaultClientVariant,
		PERMISSION_LABELS
	} from '$lib/devices'
	import type { UserPermission } from '$lib/graphql/generated/graphql'
	import { cn } from '@stump/ui/utils.js'
	import ClientCapabilities from './ClientCapabilities.svelte'

	let {
		entry,
		variant = null,
		selectedVariantId = null,
		selected = false,
		available = true,
		missingPermissions = [],
		unavailableReason = null,
		managementHref = null,
		variantAvailable,
		onclick,
		onselectvariant
	}: {
		entry: ClientCatalogEntry
		variant?: ClientAppVariant | null
		selectedVariantId?: string | null
		selected?: boolean
		available?: boolean
		missingPermissions?: readonly UserPermission[]
		unavailableReason?: string | null
		managementHref?: string | null
		variantAvailable?: (variant: ClientAppVariant) => boolean
		onclick?: () => void
		onselectvariant?: (variant: ClientAppVariant) => void
	} = $props()

	const activeVariant = $derived(variant ?? defaultClientVariant(entry))
	const variants = $derived(clientVariants(entry))
	const Icon = $derived(activeVariant?.icon ?? entry.icon)
	const evidenceTone = $derived(
		activeVariant?.evidence.label === 'Device tested'
			? 'border-primary/40 bg-primary/10 text-primary'
			: activeVariant?.evidence.label === 'Contract checked'
				? 'border-border bg-muted/70 text-foreground'
				: 'border-dashed border-muted-foreground/50 text-muted-foreground'
	)
	const maturityLabel = $derived(activeVariant?.maturity ? activeVariant.maturity : entry.maturity)
	const selectedLabel = $derived(activeVariant?.label ?? entry.title)

	function chooseVariant(next: ClientAppVariant, event: MouseEvent): void {
		event.stopPropagation()
		if (variantAvailable && !variantAvailable(next)) return
		onselectvariant?.(next)
	}

	function isVariantAvailable(next: ClientAppVariant): boolean {
		return variantAvailable?.(next) ?? true
	}
</script>

<article
	class={cn(
		'group flex h-full min-h-full min-w-0 w-full flex-col gap-2 rounded-xl border bg-card p-3 text-left transition-colors',
		available && 'hover:border-primary/60 hover:bg-muted/30',
		!available && 'cursor-not-allowed opacity-60',
		selected && 'border-primary bg-primary/5 ring-1 ring-primary/30'
	)}
	data-client-card={entry.id}
>
	<button
		type="button"
		class={cn(
			'flex min-w-0 items-start gap-2 rounded-lg text-left outline-none',
			available && 'cursor-pointer focus-visible:ring-3 focus-visible:ring-ring/50',
			!available && 'cursor-not-allowed'
		)}
		disabled={!available}
		aria-disabled={!available}
		aria-pressed={selected}
		aria-label={`${entry.title}${selected ? ', selected' : ''}${!available ? ', unavailable' : ''}`}
		{onclick}
	>
		<span class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
			<Icon class="size-4" aria-hidden="true" />
		</span>
		<span class="min-w-0 flex-1">
			<span class="flex flex-wrap items-center gap-1.5">
				<span class="truncate text-sm font-semibold">{entry.title}</span>
				{#if activeVariant}
					<Badge variant="outline" class="max-w-full shrink-0 truncate px-1.5 py-0 text-[10px] leading-4">
						{selectedLabel}
					</Badge>
				{/if}
				<Badge
					variant="outline"
					class={cn('shrink-0 px-1.5 py-0 text-[10px] leading-4', evidenceTone)}
					title={activeVariant?.evidence.detail ?? entry.evidence.detail}
					aria-label={`${activeVariant?.evidence.label ?? entry.evidence.label}: ${activeVariant?.evidence.detail ?? entry.evidence.detail}`}
				>
					{activeVariant?.evidence.label ?? entry.evidence.label}
				</Badge>
			</span>
			<span class="mt-0.5 block text-xs leading-snug text-muted-foreground">{entry.description}</span>
		</span>
	</button>
	{#if activeVariant}
		<span class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-2 gap-y-0.5 text-xs leading-snug">
			<span class="font-medium text-muted-foreground">Runs on</span>
			<span class="line-clamp-2" title={activeVariant.platforms.join(' · ')}>{activeVariant.platforms.join(' · ')}</span>
			<span class="font-medium text-muted-foreground">App</span>
			<span class="line-clamp-2" title={activeVariant.label}>{activeVariant.label}</span>
		</span>

		<span class="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground" title={activeVariant.protocol}>
			<span class="font-medium text-foreground">Connection</span>
			<span class="truncate">{activeVariant.protocol}</span>
		</span>

		<ClientCapabilities capabilities={activeVariant.capabilities} mediaFormats={activeVariant.mediaFormats} />

		<span class="flex items-center gap-1 text-[11px] text-muted-foreground">
			<span class="font-medium text-foreground">Readiness</span>
			<span class="capitalize">{maturityLabel}</span>
		</span>

		{#if activeVariant.caveat}
			<span class="line-clamp-2 text-[11px] leading-snug text-muted-foreground" title={activeVariant.caveat}>
				{activeVariant.caveat}
			</span>
		{/if}
	{/if}

	{#if variants.length > 1 || !available}
		<div class="mt-auto flex min-w-0 flex-col gap-2">
			{#if variants.length > 1}
				<div class="flex min-w-0 flex-wrap gap-1" role="group" aria-label={`Apps in ${entry.title}`}>
					{#each variants as appVariant (appVariant.id)}
						{@const VariantIcon = appVariant.icon ?? entry.icon}
						{@const appAvailable = isVariantAvailable(appVariant)}
						<button
							type="button"
							class={cn(
								'inline-flex min-h-7 min-w-0 max-w-full items-center gap-1 rounded-md border px-2 py-1 text-[11px] font-medium transition-colors outline-none focus-visible:ring-3 focus-visible:ring-ring/50',
								selectedVariantId === appVariant.id
									? 'border-primary/60 bg-primary/10 text-primary'
									: 'border-border/70 bg-background text-muted-foreground hover:border-primary/40 hover:text-foreground',
								!appAvailable && 'cursor-not-allowed opacity-50'
							)}
							data-app-variant={appVariant.id}
							aria-pressed={selectedVariantId === appVariant.id}
							aria-disabled={!appAvailable}
							disabled={!appAvailable}
							title={!appAvailable ? 'Disabled by the server' : undefined}
							onclick={(event) => chooseVariant(appVariant, event)}
						>
							<VariantIcon class="size-3 shrink-0" aria-hidden="true" />
							<span class="min-w-0 break-words text-left">{appVariant.label}</span>
						</button>
					{/each}
				</div>
			{/if}

			{#if !available}
				<div class="text-[11px] font-medium text-destructive">
					{#if unavailableReason}
						Unavailable: {unavailableReason}
					{:else}
						Missing: {missingPermissions.map((permission) => PERMISSION_LABELS[permission] ?? permission).join(', ')}
					{/if}
				</div>
				{#if managementHref}
					<a
						href={managementHref}
						class="inline-flex w-fit text-[11px] font-medium text-primary underline-offset-4 hover:underline"
						onclick={(event) => event.stopPropagation()}
					>
						Review in Components
					</a>
				{/if}
			{/if}
		</div>
	{/if}
</article>

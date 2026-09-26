<script lang="ts" module>
	/** One `label → value` row under the card header (Runs on, App, Connection, Last seen …). */
	export type ClientCardMeta = { label: string; value: string; title?: string; code?: boolean }
</script>

<script lang="ts">
	/**
	 * The one card for a client, whether it is a catalog option in the add
	 * flow or a paired device on the Devices page: icon, title, inline status
	 * marks, a two-line description, compact meta rows, then Sync/Reads chips.
	 *
	 * Catalog option: pass `onchoose`; the title button then stretches over the
	 * whole card while `footer` and the status marks stay clickable above it.
	 * Paired device: pass `heading`, `badges`, `actions`, and `children`.
	 */
	import type { LucideIcon } from '@lucide/svelte'
	import type { Snippet } from 'svelte'
	import type { HTMLAttributes } from 'svelte/elements'
	import * as Card from '@stump/ui/components/ui/card'
	import { cn } from '@stump/ui/utils.js'
	import type {
		ClientCapability,
		ClientEvidence,
		ClientMaturity,
		ClientMediaCapability
	} from '$lib/devices'
	import ClientCapabilities from './ClientCapabilities.svelte'
	import ClientStatusIcons from './ClientStatusIcons.svelte'

	let {
		icon: Icon,
		title,
		evidence = null,
		maturity = null,
		description = null,
		meta = [],
		capabilities = [],
		mediaFormats = [],
		onchoose,
		selected = false,
		available = true,
		muted = false,
		class: className,
		heading,
		badges,
		actions,
		children,
		footer,
		...rest
	}: HTMLAttributes<HTMLDivElement> & {
		icon: LucideIcon
		title: string
		evidence?: ClientEvidence | null
		maturity?: ClientMaturity | null
		description?: string | null
		meta?: readonly ClientCardMeta[]
		capabilities?: readonly ClientCapability[]
		mediaFormats?: readonly ClientMediaCapability[]
		/** Catalog option: selecting the card. */
		onchoose?: () => void
		selected?: boolean
		available?: boolean
		/** Paired device that was revoked. */
		muted?: boolean
		/** Replaces the plain title, e.g. a paired device's rename control. */
		heading?: Snippet
		/** Inline after the status marks, e.g. Enabled/Revoked. */
		badges?: Snippet
		/** Trailing header slot, e.g. an actions menu. */
		actions?: Snippet
		children?: Snippet
		/** Pinned to the bottom, e.g. app choices or read-only links. */
		footer?: Snippet
	} = $props()

	const selectable = $derived(onchoose !== undefined)
</script>

<Card.Root
	class={cn(
		'relative h-full min-w-0 gap-2 py-3 transition-[color,box-shadow,background-color]',
		selectable && available && 'hover:bg-muted/30 hover:ring-primary/50 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring/50',
		selectable && !available && 'cursor-not-allowed opacity-60',
		selected && 'bg-primary/5 ring-2 ring-primary/50 hover:ring-primary/50',
		muted && 'bg-muted/30 opacity-70',
		className
	)}
	data-selected={selected || undefined}
	{...rest}
>
	<Card.Header class="flex items-start gap-2.5 px-3">
		<span class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
			<Icon class="size-4" aria-hidden="true" />
		</span>
		<div class="min-w-0 flex-1">
			<div class="flex min-w-0 items-center gap-x-1.5">
				{#if heading}
					{@render heading()}
				{:else if selectable}
					<button
						type="button"
						class={cn(
							'min-w-0 truncate rounded-sm text-left font-semibold outline-none after:absolute after:inset-0 after:content-[""]',
							available ? 'cursor-pointer' : 'cursor-not-allowed'
						)}
						disabled={!available}
						aria-pressed={selected}
						aria-label={`${title}${selected ? ', selected' : ''}${!available ? ', unavailable' : ''}`}
						onclick={onchoose}
					>
						{title}
					</button>
				{:else}
					<h3 class="min-w-0 truncate font-semibold">{title}</h3>
				{/if}
				<ClientStatusIcons {evidence} {maturity} class="relative z-10 shrink-0" />
				{@render badges?.()}
			</div>
			{#if description}
				<p class="mt-0.5 line-clamp-2 text-xs leading-snug text-muted-foreground">{description}</p>
			{/if}
		</div>
		{#if actions}
			<div class="relative z-10 shrink-0">{@render actions()}</div>
		{/if}
	</Card.Header>

	{#if meta.length || capabilities.length || mediaFormats.length || children}
		<Card.Content class="gap-2 px-3">
			{#if meta.length}
				<dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-2 text-xs leading-tight">
					{#each meta as row (row.label)}
						<dt class="font-medium text-muted-foreground">{row.label}</dt>
						<dd class={cn('truncate', row.code && 'font-mono')} title={row.title ?? row.value}>{row.value}</dd>
					{/each}
				</dl>
			{/if}
			{#if capabilities.length || mediaFormats.length}
				<ClientCapabilities {capabilities} {mediaFormats} class="relative z-10" />
			{/if}
			{@render children?.()}
		</Card.Content>
	{/if}

	{#if footer}
		<Card.Footer class="relative z-10 mt-auto flex-col items-stretch gap-2 px-3">
			{@render footer()}
		</Card.Footer>
	{/if}
</Card.Root>

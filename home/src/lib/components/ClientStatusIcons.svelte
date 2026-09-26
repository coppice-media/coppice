<script lang="ts">
	/**
	 * The two inline status marks that follow a client title: how its support
	 * was established (evidence) and how ready it is (maturity). Each is an
	 * icon-only tooltip trigger; the text lives in the tooltip and the label.
	 */
	import type { LucideIcon } from '@lucide/svelte'
	import BadgeCheckIcon from '@lucide/svelte/icons/badge-check'
	import FlaskConicalIcon from '@lucide/svelte/icons/flask-conical'
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check'
	import * as Tooltip from '@stump/ui/components/ui/tooltip'
	import { cn } from '@stump/ui/utils.js'
	import type { ClientEvidence, ClientEvidenceLabel, ClientMaturity } from '$lib/devices'

	let {
		evidence = null,
		maturity = null,
		class: className
	}: {
		evidence?: ClientEvidence | null
		maturity?: ClientMaturity | null
		class?: string
	} = $props()

	const EVIDENCE: Record<ClientEvidenceLabel, { icon: LucideIcon; tone: string }> = {
		'Device tested': { icon: BadgeCheckIcon, tone: 'text-primary' },
		'Contract checked': { icon: ShieldCheckIcon, tone: 'text-foreground' },
		Preview: { icon: FlaskConicalIcon, tone: 'text-muted-foreground' }
	}
	const MATURITY: Record<ClientMaturity, { label: string; dot: string }> = {
		stable: { label: 'Stable', dot: 'bg-primary' },
		beta: { label: 'Beta', dot: 'bg-primary/60' },
		preview: { label: 'Preview', dot: 'bg-muted-foreground/60' },
		unverified: { label: 'Unverified', dot: 'border border-dashed border-muted-foreground' }
	}

	const TRIGGER =
		'inline-flex size-5 shrink-0 items-center justify-center rounded-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50'
</script>

{#if evidence || maturity}
	<span class={cn('inline-flex items-center', className)}>
		{#if evidence}
			{@const mark = EVIDENCE[evidence.label]}
			{@const EvidenceIcon = mark.icon}
			<Tooltip.Root ignoreNonKeyboardFocus>
				<Tooltip.Trigger class={TRIGGER} aria-label={`${evidence.label}: ${evidence.detail}`}>
					<EvidenceIcon class={cn('size-3.5', mark.tone)} aria-hidden="true" />
				</Tooltip.Trigger>
				<Tooltip.Content class="max-w-64">
					<span><span class="font-medium">{evidence.label}.</span> {evidence.detail}</span>
				</Tooltip.Content>
			</Tooltip.Root>
		{/if}
		{#if maturity}
			{@const readiness = MATURITY[maturity]}
			<Tooltip.Root ignoreNonKeyboardFocus>
				<Tooltip.Trigger class={TRIGGER} aria-label={`Readiness: ${readiness.label}`}>
					<span class={cn('size-2 rounded-full', readiness.dot)} aria-hidden="true"></span>
				</Tooltip.Trigger>
				<Tooltip.Content>Readiness: {readiness.label}</Tooltip.Content>
			</Tooltip.Root>
		{/if}
	</span>
{/if}

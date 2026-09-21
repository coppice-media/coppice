<script lang="ts">
	import type { ClientCapability, ClientCapabilityStatus, ClientMediaCapability } from '$lib/devices'
	import { cn } from '@stump/ui/utils.js'

	let {
		capabilities,
		mediaFormats
	}: {
		capabilities: readonly ClientCapability[]
		mediaFormats: readonly ClientMediaCapability[]
	} = $props()

	const syncCapabilities = $derived(capabilities.filter((item) => item.group === 'sync'))
	const readCapabilities = $derived(capabilities.filter((item) => item.group === 'reads'))

	function statusLabel(status: ClientCapabilityStatus): string {
		switch (status) {
			case 'full':
				return 'supported'
			case 'partial':
				return 'partial support'
			case 'unsupported':
				return 'unsupported'
			case 'unverified':
				return 'unverified'
		}
	}

	function statusMarker(status: ClientCapabilityStatus): string {
		switch (status) {
			case 'full':
				return ''
			case 'partial':
				return '~'
			case 'unsupported':
				return '–'
			case 'unverified':
				return '?'
		}
	}

	function statusClasses(status: ClientCapabilityStatus): string {
		return cn(
			'inline-flex min-h-5 items-center gap-1 rounded-md border px-1.5 py-0.5 text-[11px] leading-none',
			status === 'full' && 'border-border/70 bg-muted/50 text-foreground',
			status === 'partial' && 'border-dashed border-muted-foreground/50 text-muted-foreground',
			status === 'unsupported' && 'border-destructive/40 bg-destructive/5 text-muted-foreground',
			status === 'unverified' && 'border-dotted border-muted-foreground/50 text-muted-foreground'
		)
	}
</script>

<span class="flex flex-col gap-1.5" aria-label="Capabilities">
	{#if syncCapabilities.length}
		<span class="flex flex-wrap items-center gap-1.5" aria-label="Sync capabilities">
			<span class="mr-0.5 text-[10px] font-semibold tracking-wide text-muted-foreground uppercase">Sync</span>
			{#each syncCapabilities as capability (capability.id)}
				{@const Icon = capability.icon}
				<span
					class={statusClasses(capability.status)}
					aria-label={`${capability.label}: ${statusLabel(capability.status)}${capability.detail ? `. ${capability.detail}` : ''}`}
					title={capability.detail ?? `${capability.label}: ${statusLabel(capability.status)}`}
				>
					<Icon class="size-3" aria-hidden="true" />
					<span>{capability.label}</span>
					{#if capability.status !== 'full'}
						<span class="sr-only">{statusLabel(capability.status)}</span>
						<span aria-hidden="true">{statusMarker(capability.status)}</span>
					{/if}
				</span>
			{/each}
		</span>
	{/if}

	{#if readCapabilities.length || mediaFormats.length}
		<span class="flex flex-wrap items-center gap-1.5" aria-label="Reading capabilities and formats">
			<span class="mr-0.5 text-[10px] font-semibold tracking-wide text-muted-foreground uppercase">Reads</span>
			{#each readCapabilities as capability (capability.id)}
				{@const Icon = capability.icon}
				<span
					class={statusClasses(capability.status)}
					aria-label={`${capability.label}: ${statusLabel(capability.status)}${capability.detail ? `. ${capability.detail}` : ''}`}
					title={capability.detail ?? `${capability.label}: ${statusLabel(capability.status)}`}
				>
					<Icon class="size-3" aria-hidden="true" />
					<span>{capability.label}</span>
					{#if capability.status !== 'full'}
						<span class="sr-only">{statusLabel(capability.status)}</span>
						<span aria-hidden="true">{statusMarker(capability.status)}</span>
					{/if}
				</span>
			{/each}
			{#each mediaFormats as format (format.format)}
				{@const Icon = format.icon}
				<span
					class={statusClasses(format.status)}
					aria-label={`${format.label}: ${statusLabel(format.status)}${format.detail ? `. ${format.detail}` : ''}`}
					title={format.detail ?? `${format.label}: ${statusLabel(format.status)}`}
				>
					<Icon class="size-3" aria-hidden="true" />
					<span>{format.label}</span>
					{#if format.status !== 'full'}
						<span class="sr-only">{statusLabel(format.status)}</span>
						<span aria-hidden="true">{statusMarker(format.status)}</span>
					{/if}
				</span>
			{/each}
		</span>
	{/if}
</span>

<script lang="ts">
	import { browser } from '$app/environment'
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query'
	import AlertTriangleIcon from '@lucide/svelte/icons/alert-triangle'
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2'
	import CircleOffIcon from '@lucide/svelte/icons/circle-off'
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw'
	import ServerIcon from '@lucide/svelte/icons/server'
	import { toast } from 'svelte-sonner'
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert'
	import { Badge } from '@stump/ui/components/ui/badge'
	import { Button } from '@stump/ui/components/ui/button'
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card'
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty'
	import { Label } from '@stump/ui/components/ui/label'
	import { PageHeader } from '@stump/ui/components/ui/page-header'
	import { Separator } from '@stump/ui/components/ui/separator'
	import { Skeleton } from '@stump/ui/components/ui/skeleton'
	import { Switch } from '@stump/ui/components/ui/switch'
	import { request } from '@stump/ui/graphql/client'
	import { errorMessage } from '@stump/ui/utils/errors.js'
	import {
		RuntimeComponentsDocument,
		SetRuntimeComponentEnabledDocument
	} from '$lib/graphql/generated/graphql'
	import { bytesLabel, absoluteTime, countLabel } from '$lib/format'
	import { getHomeSession } from '$lib/session.svelte'

	type RuntimeGauge = {
		name: string
		kind: string
		value: unknown
	}

	type RuntimeComponent = {
		key: string
		label: string
		description: string
		category: string
		compiled: boolean
		desiredEnabled: boolean
		effectiveEnabled: boolean
		transitionMode: string
		transitionReason: string
		dependencies: readonly string[]
		health: string
		restartRequired: boolean
		usageStatus: string
		usageEvidence?: string | null
		activityCount?: number | null
		lastActivityAt?: string | null
		lastTransitionAt?: string | null
		lastError?: string | null
		ownedGauges: readonly RuntimeGauge[]
	}

	type RuntimeMemory = {
		totalProcessRssBytes: unknown
		rssAvailable: boolean
		rssUnavailableReason?: string | null
		anonymousPssBytes?: unknown
		fileBackedPssBytes?: unknown
		privateDirtyBytes?: unknown
		memoryBreakdownAvailable: boolean
		memoryBreakdownUnavailableReason?: string | null
	}

	const session = getHomeSession()
	const queryClient = useQueryClient()
	const isOwner = $derived(Boolean(session.user?.isServerOwner))

	const componentsQuery = createQuery(() => ({
		queryKey: ['runtime-components'],
		queryFn: () => request(RuntimeComponentsDocument, {}),
		enabled: browser && isOwner
	}))

	const components = $derived(
		(componentsQuery.data?.runtimeComponents ?? []) as RuntimeComponent[]
	)
	const runtimeMemory = $derived(
		(componentsQuery.data?.runtimeMemory ?? null) as RuntimeMemory | null
	)

	const toggle = createMutation(() => ({
		mutationFn: ({ key, enabled }: { key: string; enabled: boolean }) =>
			request(SetRuntimeComponentEnabledDocument, { key, enabled }),
		onSuccess: (_result, variables) => {
			void queryClient.invalidateQueries({ queryKey: ['runtime-components'] })
			void queryClient.invalidateQueries({ queryKey: ['device-capabilities'] })
			toast.success(
				variables.enabled
					? 'Component enablement requested.'
					: 'Component disablement requested.'
			)
		},
		onError: (error) => toast.error(errorMessage(error))
	}))
	function toggleComponent(component: RuntimeComponent, enabled: boolean): void {
		if (!component.compiled || toggle.isPending) return
		toggle.mutate({ key: component.key, enabled })
	}

	function numberValue(value: unknown): number | null {
		if (typeof value === 'number' && Number.isFinite(value)) return value
		if (typeof value === 'bigint') return Number(value)
		if (typeof value === 'string' && value.trim() !== '') {
			const parsed = Number(value)
			return Number.isFinite(parsed) ? parsed : null
		}
		return null
	}

	function gaugeIsBytes(kind: unknown): boolean {
		return String(kind).toUpperCase() === 'BYTES'
	}

	function gaugeValue(gauge: RuntimeGauge): string {
		const value = numberValue(gauge.value)
		if (value === null) return 'Unavailable'
		return gaugeIsBytes(gauge.kind) ? bytesLabel(value) : countLabel(value)
	}

	function gaugeUnit(gauge: RuntimeGauge): string {
		return gaugeIsBytes(gauge.kind)
			? 'bytes · component-owned (not total RAM)'
			: 'count · component-owned entries/items'
	}

	function memoryValue(): string | null {
		if (!runtimeMemory?.rssAvailable) return null
		const value = numberValue(runtimeMemory.totalProcessRssBytes)
		return value === null ? null : bytesLabel(value)
	}
	function processMemoryValue(value: unknown): string | null {
		const bytes = numberValue(value)
		return bytes === null ? null : bytesLabel(bytes)
	}

	function usageLabel(status: string): string {
		switch (status.toUpperCase()) {
			case 'USED':
				return 'Used'
			case 'UNUSED':
				return 'Unused'
			default:
				return 'Usage unknown'
		}
	}

	function usageVariant(status: string): 'secondary' | 'outline' | 'destructive' {
		switch (status.toUpperCase()) {
			case 'USED':
				return 'secondary'
			case 'UNUSED':
				return 'destructive'
			default:
				return 'outline'
		}
	}
	function transitionLabel(component: RuntimeComponent): string {
		if (component.transitionMode.toUpperCase() === 'HOT') return 'Hot transition'
		return component.restartRequired ? 'Restart required' : 'Restart boundary'
	}
</script>

<svelte:head>
	<title>Components · Coppice</title>
	<meta
		name="description"
		content="Inspect server component health, capability state, and honest owned gauges."
	/>
</svelte:head>

<div class="flex flex-col gap-8">
	<PageHeader
		title="Components"
		description="Inspect compiled integrations, desired and effective state, lifecycle requirements, and component-owned gauges."
	/>

	{#if !session.user}
		<Alert aria-live="polite">
			<AlertTitle>Checking account access</AlertTitle>
			<AlertDescription>Loading the current account before showing server controls.</AlertDescription>
		</Alert>
	{:else if !isOwner}
		<Alert>
			<AlertTitle>Server owner access required</AlertTitle>
			<AlertDescription>
				Component controls are limited to the server owner. Ask the owner to review disabled integrations or restart-required changes.
			</AlertDescription>
		</Alert>
	{:else if componentsQuery.isPending}
		<div class="grid gap-5 xl:grid-cols-2" aria-label="Loading server components">
			<Skeleton class="h-72 rounded-xl" />
			<Skeleton class="h-72 rounded-xl" />
			<Skeleton class="h-72 rounded-xl" />
			<Skeleton class="h-72 rounded-xl" />
		</div>
	{:else if componentsQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load server components</AlertTitle>
			<AlertDescription>
				{componentsQuery.error instanceof Error ? componentsQuery.error.message : 'Request failed.'}
			</AlertDescription>
			<Button type="button" variant="outline" class="mt-3" onclick={() => componentsQuery.refetch()}>
				<RotateCcwIcon data-icon="inline-start" />
				Retry
			</Button>
		</Alert>
	{:else if components.length === 0}
		<Empty class="rounded-xl border border-dashed bg-card">
			<EmptyHeader>
				<EmptyTitle>No component descriptors</EmptyTitle>
				<EmptyDescription>
					This server did not publish any runtime component descriptors. Client setup remains governed by the server capabilities endpoint.
				</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else}
		<Card>
			<CardHeader>
				<div class="flex flex-wrap items-start justify-between gap-3">
					<div>
						<CardTitle class="text-base">Total server memory</CardTitle>
						<CardDescription>
							RSS is a process-wide measurement and cannot be attributed to individual components.
						</CardDescription>
					</div>
					<ServerIcon class="size-5 text-muted-foreground" aria-hidden="true" />
				</div>
			</CardHeader>
			<CardContent>
				{#if memoryValue()}
					<p class="text-3xl font-semibold tracking-tight">{memoryValue()}</p>
					<p class="mt-1 text-xs text-muted-foreground">Total process RSS · unattributed across components</p>
				{:else}
					<p class="font-medium">Total process RSS unavailable</p>
					<p class="mt-1 text-xs text-muted-foreground">
						{runtimeMemory?.rssUnavailableReason ?? 'The server did not expose a safe RSS measurement.'}
					</p>
				{/if}
				{#if runtimeMemory?.memoryBreakdownAvailable}
					<div class="mt-4 grid gap-2 sm:grid-cols-3">
						<div class="rounded-lg border bg-muted/20 p-3">
							<p class="text-xs text-muted-foreground">Anonymous PSS</p>
							<p class="mt-1 font-semibold">{processMemoryValue(runtimeMemory?.anonymousPssBytes) ?? 'Unavailable'}</p>
							<p class="mt-0.5 text-[11px] text-muted-foreground">Process-wide allocator/runtime pages</p>
						</div>
						<div class="rounded-lg border bg-muted/20 p-3">
							<p class="text-xs text-muted-foreground">File-backed PSS</p>
							<p class="mt-1 font-semibold">{processMemoryValue(runtimeMemory?.fileBackedPssBytes) ?? 'Unavailable'}</p>
							<p class="mt-0.5 text-[11px] text-muted-foreground">Process-wide code/shared mappings</p>
						</div>
						<div class="rounded-lg border bg-muted/20 p-3">
							<p class="text-xs text-muted-foreground">Private dirty</p>
							<p class="mt-1 font-semibold">{processMemoryValue(runtimeMemory?.privateDirtyBytes) ?? 'Unavailable'}</p>
							<p class="mt-0.5 text-[11px] text-muted-foreground">Process-wide modified private pages</p>
						</div>
					</div>
				{:else if runtimeMemory?.memoryBreakdownUnavailableReason}
					<p class="mt-4 text-xs text-muted-foreground">{runtimeMemory?.memoryBreakdownUnavailableReason}</p>
				{/if}
			</CardContent>
		</Card>

		{#if toggle.isError}
			<Alert variant="destructive">
				<AlertTitle>Component change failed</AlertTitle>
				<AlertDescription>
					{toggle.error instanceof Error ? toggle.error.message : 'The requested state could not be saved.'}
				</AlertDescription>
			</Alert>
		{/if}

		<div class="grid gap-5 xl:grid-cols-2" aria-label="Server components">
			{#each components as component (component.key)}
				<Card>
					<CardHeader>
						<div class="flex flex-wrap items-start justify-between gap-3">
							<div class="min-w-0">
								<CardTitle class="text-base">{component.label}</CardTitle>
								<CardDescription>
									{component.category} · <code>{component.key}</code>
								</CardDescription>
								<p class="mt-2 text-sm text-foreground">{component.description}</p>
							</div>
							<div class="flex items-center gap-2">
								<Badge variant={component.compiled ? 'secondary' : 'outline'}>
									{component.compiled ? 'Compiled' : 'Not compiled'}
								</Badge>
								{#if component.compiled}
									<Label class="flex items-center gap-2 text-xs font-normal" for={`component-${component.key}`}>
										<Switch
											id={`component-${component.key}`}
											checked={component.desiredEnabled}
											disabled={toggle.isPending}
											onCheckedChange={(enabled) => toggleComponent(component, enabled)}
										/>
										<span>Desired</span>
									</Label>
								{:else}
									<Badge variant="outline">Unavailable</Badge>
								{/if}
							</div>
						</div>
					</CardHeader>
					<CardContent class="flex flex-col gap-4">
						<div class="grid gap-3 sm:grid-cols-2">
							<div class="rounded-lg border bg-muted/20 p-3">
								<p class="text-xs text-muted-foreground">Desired state</p>
								<p class="mt-1 flex items-center gap-1.5 font-medium">
									{#if component.desiredEnabled}
										<CheckCircle2Icon class="size-4" aria-hidden="true" /> Enabled
									{:else}
										<CircleOffIcon class="size-4" aria-hidden="true" /> Disabled
									{/if}
								</p>
							</div>
							<div class="rounded-lg border bg-muted/20 p-3">
								<p class="text-xs text-muted-foreground">Effective state</p>
								<p class="mt-1 flex items-center gap-1.5 font-medium">
									{#if component.effectiveEnabled}
										<CheckCircle2Icon class="size-4" aria-hidden="true" /> Enabled
									{:else}
										<CircleOffIcon class="size-4" aria-hidden="true" /> Disabled
									{/if}
								</p>
							</div>
						</div>

						<div class="flex flex-wrap items-center gap-2">
							<Badge variant={component.health.toLowerCase() === 'active' ? 'secondary' : 'outline'}>
								Health: {component.health || 'Unknown'}
							</Badge>
							<Badge variant="outline">{transitionLabel(component)}</Badge>
						</div>

						{#if component.transitionMode === 'RESTART' || component.restartRequired}
							<Alert>
								<AlertTriangleIcon data-icon="inline-start" aria-hidden="true" />
								<AlertTitle>{component.restartRequired ? 'Restart required' : 'Restart boundary'}</AlertTitle>
								<AlertDescription>{component.transitionReason}</AlertDescription>
							</Alert>
						{:else}
							<p class="text-xs text-muted-foreground">
								<strong class="text-foreground">Hot transition.</strong>
								{component.transitionReason}
							</p>
						{/if}

						<div class="rounded-lg border bg-muted/20 p-3">
							<div class="flex flex-wrap items-center gap-2">
								<p class="text-xs font-medium text-muted-foreground">Usage evidence</p>
								<Badge variant={usageVariant(component.usageStatus)}>
									{usageLabel(component.usageStatus)}
								</Badge>
							</div>
							<p class="mt-1 text-sm">{component.usageEvidence ?? 'Usage unknown: no evidence was reported.'}</p>
							{#if component.activityCount !== null && component.activityCount !== undefined}
								<p class="mt-1 text-xs text-muted-foreground">
									Observed activity count: {countLabel(component.activityCount)}
									{#if component.lastActivityAt} · last {absoluteTime(component.lastActivityAt)}{/if}
								</p>
							{/if}
						</div>

						{#if component.dependencies.length}
							<div>
								<p class="text-xs font-medium text-muted-foreground">Dependencies</p>
								<p class="mt-1 text-sm">{component.dependencies.join(' · ')}</p>
							</div>
						{/if}

						{#if component.lastTransitionAt}
							<p class="text-xs text-muted-foreground">Last transition {absoluteTime(component.lastTransitionAt)}</p>
						{/if}

						{#if component.lastError}
							<Alert variant="destructive">
								<AlertTitle>Component error</AlertTitle>
								<AlertDescription>{component.lastError}</AlertDescription>
							</Alert>
						{/if}

						{#if component.ownedGauges.length}
							<Separator />
							<div>
								<p class="text-sm font-medium">Owned gauges</p>
								<p class="mt-1 text-xs text-muted-foreground">
									Only exact bytes or counts reported by this component are shown; these are not allocator or total-RAM estimates.
								</p>
								<dl class="mt-3 grid gap-2 sm:grid-cols-2">
									{#each component.ownedGauges as gauge (gauge.name)}
										<div class="rounded-lg border bg-muted/20 p-3">
											<dt class="truncate text-xs text-muted-foreground" title={gauge.name}>{gauge.name}</dt>
											<dd class="mt-1 text-lg font-semibold">{gaugeValue(gauge)}</dd>
											<p class="mt-0.5 text-[11px] text-muted-foreground">{gaugeUnit(gauge)}</p>
										</div>
									{/each}
								</dl>
							</div>
						{:else}
							<p class="text-xs text-muted-foreground">
								No component-owned bytes or counts are instrumented for this component. Process RSS above remains unattributed.
							</p>
						{/if}
					</CardContent>
				</Card>
			{/each}
		</div>
	{/if}
</div>

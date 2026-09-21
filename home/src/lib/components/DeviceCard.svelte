<script lang="ts">
	import { browser } from '$app/environment'
	import { resolve } from '$app/paths'
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query'
	import ActivityIcon from '@lucide/svelte/icons/activity'
	import BookOpenTextIcon from '@lucide/svelte/icons/book-open-text'
	import EllipsisVerticalIcon from '@lucide/svelte/icons/ellipsis-vertical'
	import HighlighterIcon from '@lucide/svelte/icons/highlighter'
	import LayersIcon from '@lucide/svelte/icons/layers'
	import LibraryIcon from '@lucide/svelte/icons/library'
	import PencilIcon from '@lucide/svelte/icons/pencil'
	import { toast } from 'svelte-sonner'
	import * as AlertDialog from '@stump/ui/components/ui/alert-dialog'
	import { Badge } from '@stump/ui/components/ui/badge'
	import { Button, buttonVariants } from '@stump/ui/components/ui/button'
	import * as Card from '@stump/ui/components/ui/card'
	import { Checkbox } from '@stump/ui/components/ui/checkbox'
	import * as Dialog from '@stump/ui/components/ui/dialog'
	import * as DropdownMenu from '@stump/ui/components/ui/dropdown-menu'
	import { Input } from '@stump/ui/components/ui/input'
	import { Label } from '@stump/ui/components/ui/label'
	import * as Select from '@stump/ui/components/ui/select'
	import { Switch } from '@stump/ui/components/ui/switch'
	import { request } from '@stump/ui/graphql/client'
	import { cn } from '@stump/ui/utils.js'
	import { errorMessage } from '@stump/ui/utils/errors.js'
	import {
		ConsoleLibraryOptionsDocument,
		RenameDeviceDocument,
		RevokeDeviceDocument,
		RotateDeviceCredentialDocument,
		SetDeviceLibraryScopeDocument,
		SetDeviceTransformProfileDocument
	} from '$lib/graphql/generated/graphql'
	import {
		DEVICE_KIND_ICONS,
		DEVICE_KIND_LABELS,
		NO_PRESET,
		PASSTHROUGH_AUDIO,
		PROTOCOL_LABELS,
		TRANSFORMABLE_KINDS,
		TRANSFORM_PRESETS,
		audioDeliveryOf,
		libraryScopeSummary,
		presetOf,
		protocolForKind,
		syncSentence,
		type Device,
		type IssuedCredential
	} from '$lib/devices'
	import { absoluteTime, relativeTime } from '$lib/format'
	import CredentialReveal from './CredentialReveal.svelte'
	import CrossPointDeliveryQueue from './CrossPointDeliveryQueue.svelte'
	import CrossPointTargetSetup from './CrossPointTargetSetup.svelte'

	let { device, now }: { device: Device; now: Date } = $props()

	type Panel = 'libraries' | 'preset'
	type Confirmation = 'rotate' | 'revoke'

	const queryClient = useQueryClient()
	const revoked = $derived(!!device.revokedAt)
	const KindIcon = $derived(DEVICE_KIND_ICONS[device.kind])
	const protocol = $derived(device.credential?.protocol ?? protocolForKind(device.kind))
	const protocolLabel = $derived(PROTOCOL_LABELS[protocol])
	const preset = $derived(presetOf(device.transformProfile))
	const presetEntry = $derived(TRANSFORM_PRESETS.find((candidate) => candidate.name === preset))
	const presetChip = $derived(
		presetEntry?.short ?? (preset === NO_PRESET ? 'Server default' : `Custom (${preset})`)
	)
	const audioDelivery = $derived(audioDeliveryOf(device.transformProfile))
	const sentence = $derived(syncSentence(device.lastSyncSummary))
	const transformable = $derived(!!TRANSFORMABLE_KINDS[device.kind])
	const annotationHref = $derived(`${resolve('/annotations')}?device=${encodeURIComponent(device.id)}`)
	const readingHref = $derived(`${resolve('/reading')}?device=${encodeURIComponent(device.id)}`)
	const activityHref = $derived(`${readingHref}#reading-summary-heading`)
	const progressHref = $derived(`${readingHref}#current-reading-heading`)

	let renaming = $state(false)
	let draftName = $state('')
	let panel = $state<Panel | null>(null)
	let confirmation = $state<Confirmation | null>(null)
	let rotated = $state<IssuedCredential | null>(null)
	let rotatedOpen = $state(false)
	let rotatedSaved = $state(false)

	let draftRestricted = $state(false)
	let draftScope = $state<string[]>([])
	let draftPreset = $state(NO_PRESET)

	const librariesQuery = createQuery(() => ({
		queryKey: ['libraryOptions'],
		queryFn: () => request(ConsoleLibraryOptionsDocument, {}),
		enabled: browser && panel === 'libraries'
	}))
	const libraries = $derived(librariesQuery.data?.libraries.nodes ?? [])

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['devices'] })
	}

	const rename = createMutation(() => ({
		mutationFn: (name: string) => request(RenameDeviceDocument, { id: device.id, name }),
		onSuccess: () => {
			renaming = false
			invalidate()
		},
		onError: (error) => toast.error(errorMessage(error))
	}))
	const rotate = createMutation(() => ({
		mutationFn: () => request(RotateDeviceCredentialDocument, { id: device.id }),
		onSuccess: (result) => {
			rotated = result.rotateDeviceCredential
			rotatedSaved = false
			rotatedOpen = true
			invalidate()
		},
		onError: (error) => toast.error(errorMessage(error))
	}))
	const revoke = createMutation(() => ({
		mutationFn: () => request(RevokeDeviceDocument, { id: device.id }),
		onSuccess: () => {
			toast.success(`${device.name} can no longer sign in.`)
			invalidate()
		},
		onError: (error) => toast.error(errorMessage(error))
	}))
	const setPreset = createMutation(() => ({
		mutationFn: (name: string) =>
			request(SetDeviceTransformProfileDocument, {
				id: device.id,
				profile: name === NO_PRESET ? null : { preset: name }
			}),
		onSuccess: () => {
			panel = null
			toast.success(`Comic preset saved for ${device.name}.`)
			invalidate()
		},
		onError: (error) => toast.error(errorMessage(error))
	}))
	const setScope = createMutation(() => ({
		mutationFn: (libraryIds: string[] | null) =>
			request(SetDeviceLibraryScopeDocument, { id: device.id, libraryIds }),
		onSuccess: () => {
			panel = null
			toast.success(`Visible libraries saved for ${device.name}.`)
			invalidate()
		},
		onError: (error) => toast.error(errorMessage(error))
	}))

	function openPanel(which: Panel): void {
		draftRestricted = !!device.libraryScope
		draftScope = device.libraryScope ? [...device.libraryScope] : []
		draftPreset = preset
		panel = which
	}

	function toggleLibrary(id: string, checked: boolean): void {
		draftScope = checked ? [...draftScope, id] : draftScope.filter((entry) => entry !== id)
	}

	function saveScope(event: SubmitEvent): void {
		event.preventDefault()
		setScope.mutate(draftRestricted ? draftScope : null)
	}

	function savePreset(event: SubmitEvent): void {
		event.preventDefault()
		setPreset.mutate(draftPreset)
	}

	function startRename(): void {
		draftName = device.name
		renaming = true
	}

	function submitRename(event: SubmitEvent): void {
		event.preventDefault()
		const name = draftName.trim()
		if (!name || name === device.name) {
			renaming = false
			return
		}
		rename.mutate(name)
	}

	function runConfirmed(): void {
		if (confirmation === 'rotate') rotate.mutate()
		if (confirmation === 'revoke') revoke.mutate()
		confirmation = null
	}
</script>

<Card.Root
	size="sm"
	class={cn('h-full min-h-0 gap-0', revoked && 'bg-muted/30 opacity-70 ring-dashed')}
	data-device-id={device.id}
>
	<Card.Header class="!flex flex-col gap-3">
		<div class="flex min-w-0 items-start gap-3">
			<span class="flex size-10 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
				<KindIcon class="size-5" aria-hidden="true" />
			</span>
			<div class="min-w-0 flex-1">
				{#if renaming}
					<form class="flex flex-wrap items-center gap-2" onsubmit={submitRename}>
						<Input
							bind:value={draftName}
							maxlength={100}
							aria-label="Device name"
							autofocus
							class="h-8 min-w-0 flex-1"
							onkeydown={(event) => {
								if (event.key === 'Escape') renaming = false
							}}
						/>
						<Button type="submit" size="sm" disabled={rename.isPending}>Save</Button>
						<Button type="button" size="sm" variant="ghost" onclick={() => (renaming = false)}>Cancel</Button>
					</form>
				{:else}
					<div class="flex min-w-0 flex-wrap items-center gap-1.5">
						{#if revoked}
							<h3 class="min-w-0 truncate text-base font-semibold">{device.name}</h3>
						{:else}
							<button
								type="button"
								class="group/name flex min-w-0 items-center gap-1.5 rounded-md text-left outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
								onclick={startRename}
								aria-label={`Rename ${device.name}`}
							>
								<h3 class="truncate text-base font-semibold">{device.name}</h3>
								<PencilIcon
									class="size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/name:opacity-100 group-focus-visible/name:opacity-100"
									aria-hidden="true"
								/>
							</button>
						{/if}
						<Badge variant="outline">{DEVICE_KIND_LABELS[device.kind]}</Badge>
						<Badge variant="outline" title="Protocol the device uses">{protocolLabel}</Badge>
						{#if revoked}
							<Badge variant="destructive">Revoked</Badge>
						{:else}
							<Badge variant="secondary">Enabled</Badge>
						{/if}
					</div>
					<div class="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
						{#if device.credential}
							<code class="font-mono" title="Primary credential hint">{device.credential.secretHint}</code>
						{:else if !revoked}
							<span>No credential</span>
						{/if}
						<span>Added {relativeTime(device.createdAt, now)}</span>
					</div>
				{/if}
			</div>
			<div class="shrink-0">
				<DropdownMenu.Root>
					<DropdownMenu.Trigger>
						{#snippet child({ props })}
							<Button {...props} variant="ghost" size="icon-sm" aria-label={`Actions for ${device.name}`}>
								<EllipsisVerticalIcon />
							</Button>
						{/snippet}
					</DropdownMenu.Trigger>
					<DropdownMenu.Content align="end" class="min-w-52">
						<DropdownMenu.Group>
							<DropdownMenu.Item onSelect={startRename}>Rename</DropdownMenu.Item>
							{#if !revoked && device.credential}
								<DropdownMenu.Item onSelect={() => (confirmation = 'rotate')}>Rotate credential</DropdownMenu.Item>
							{/if}
						</DropdownMenu.Group>
						{#if !revoked}
							<DropdownMenu.Separator />
							<DropdownMenu.Group>
								<DropdownMenu.Item onSelect={() => openPanel('libraries')}>Visible libraries</DropdownMenu.Item>
								{#if transformable}
									<DropdownMenu.Item onSelect={() => openPanel('preset')}>Comic preset</DropdownMenu.Item>
								{/if}
							</DropdownMenu.Group>
							<DropdownMenu.Separator />
							<DropdownMenu.Item variant="destructive" onSelect={() => (confirmation = 'revoke')}>Revoke</DropdownMenu.Item>
						{/if}
					</DropdownMenu.Content>
				</DropdownMenu.Root>
			</div>
		</div>

		<div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
			<span title={device.lastSeenAt ? absoluteTime(device.lastSeenAt) : undefined}>
				Last seen <span class="font-medium text-foreground">{device.lastSeenAt ? relativeTime(device.lastSeenAt, now) : 'Never'}</span>
			</span>
			<span title={device.lastSyncAt ? absoluteTime(device.lastSyncAt) : undefined}>
				Last sync <span class="font-medium text-foreground">{device.lastSyncAt ? relativeTime(device.lastSyncAt, now) : 'Never'}</span>
			</span>
		</div>
	</Card.Header>

	<Card.Content class="flex min-h-0 flex-1 flex-col gap-3 pt-0">
		{#if sentence}
			<p class="text-sm text-muted-foreground">
				Sync summary: <span class="font-medium text-foreground">{sentence}</span>
			</p>
		{/if}

		<div class="flex flex-wrap gap-1.5" aria-label="Device scope and delivery">
			<Badge variant="secondary" title="Libraries this device can see">
				<LibraryIcon aria-hidden="true" />
				{libraryScopeSummary(device.libraryScope)}
			</Badge>
			{#if transformable}
				<Badge variant="secondary" title={presetEntry?.label ?? 'Comic transform preset'}>
					<LayersIcon aria-hidden="true" />
					{presetChip}
				</Badge>
			{/if}
			{#if audioDelivery !== PASSTHROUGH_AUDIO}
				<Badge variant="secondary" title="Audiobooks are transcoded on demand for this device">
					{audioDelivery}
				</Badge>
			{/if}
		</div>
		{#if String(device.kind) === 'CROSSPOINT' && !revoked}
			<details class="rounded-xl border bg-background/40">
				<summary class="cursor-pointer list-none px-3 py-2.5 text-sm font-medium outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
					<span class="flex items-center justify-between gap-3">
						<span>CrossPoint setup and queue</span>
						<span class="text-xs font-normal text-muted-foreground">LAN delivery</span>
					</span>
				</summary>
				<div class="flex flex-col gap-3 border-t p-3">
					<CrossPointTargetSetup deviceId={device.id} />
					<CrossPointDeliveryQueue deviceId={device.id} />
				</div>
			</details>
		{/if}

		{#if revoked}
			<p class="text-xs text-muted-foreground">
				Revoked {relativeTime(device.revokedAt, now)}. Its credential no longer works; add the client again to reconnect it.
			</p>
		{/if}

		<nav class="mt-auto flex flex-wrap items-center gap-1.5 border-t pt-3" aria-label={`Read-only links for ${device.name}`}>
			<a class={cn(buttonVariants({ variant: 'ghost', size: 'sm' }), 'h-8 gap-1.5 px-2 text-xs')} href={activityHref}>
				<ActivityIcon class="size-3.5" aria-hidden="true" />
				Activity
			</a>
			<a class={cn(buttonVariants({ variant: 'ghost', size: 'sm' }), 'h-8 gap-1.5 px-2 text-xs')} href={progressHref}>
				<BookOpenTextIcon class="size-3.5" aria-hidden="true" />
				Progress
			</a>
			<a
				class={cn(buttonVariants({ variant: 'ghost', size: 'sm' }), 'h-8 gap-1.5 px-2 text-xs')}
				href={annotationHref}
				title={`Annotations synced from ${device.name}; empty when this device has no annotation lane`}
			>
				<HighlighterIcon class="size-3.5" aria-hidden="true" />
				Annotations
			</a>
		</nav>
	</Card.Content>
</Card.Root>

<AlertDialog.Root
	open={confirmation !== null}
	onOpenChange={(value) => {
		if (!value) confirmation = null
	}}
>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>
				{confirmation === 'rotate' ? `Rotate the credential for ${device.name}?` : `Revoke ${device.name}?`}
			</AlertDialog.Title>
			<AlertDialog.Description>
				{#if confirmation === 'rotate'}
					The current secret stops working immediately and a new one is shown once. Reconfigure the device with it.
				{:else}
					Its credential stops working immediately. The device stays listed as revoked for its history; to reconnect it, add the client again.
				{/if}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={rotate.isPending || revoke.isPending}>Cancel</AlertDialog.Cancel>
			<AlertDialog.Action
				class={confirmation === 'revoke' ? buttonVariants({ variant: 'destructive' }) : undefined}
				onclick={runConfirmed}
				disabled={rotate.isPending || revoke.isPending}
			>
				{#if rotate.isPending}
					Rotating…
				{:else if revoke.isPending}
					Revoking…
				{:else}
					{confirmation === 'rotate' ? 'Rotate credential' : 'Revoke'}
				{/if}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<Dialog.Root bind:open={rotatedOpen}>
	<Dialog.Content
		class="flex max-h-[90vh] flex-col overflow-hidden sm:max-w-2xl"
		showCloseButton={false}
		escapeKeydownBehavior="ignore"
		interactOutsideBehavior="ignore"
	>
		<Dialog.Header>
			<Dialog.Title>New credential for {device.name}</Dialog.Title>
			<Dialog.Description>The previous secret no longer works. Copy the new one first, then update the device.</Dialog.Description>
		</Dialog.Header>
		{#if rotated}
			<div class="min-h-0 flex-1 overflow-y-auto pr-1">
				<CredentialReveal issued={rotated} bind:saved={rotatedSaved} />
			</div>
		{/if}
		<Dialog.Footer>
			<Button onclick={() => (rotatedOpen = false)} disabled={!rotatedSaved}>Done</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root
	open={panel !== null}
	onOpenChange={(value) => {
		if (!value) panel = null
	}}
>
	<Dialog.Content class="sm:max-w-md">
		{#if panel === 'libraries'}
			<form class="flex flex-col gap-5" onsubmit={saveScope}>
				<Dialog.Header>
					<Dialog.Title>Visible libraries</Dialog.Title>
					<Dialog.Description>A scope only narrows what {device.name} can see; it never grants a library you cannot open yourself.</Dialog.Description>
				</Dialog.Header>
				<div class="flex items-center justify-between gap-4">
					<Label for={`restrict-${device.id}`} class="flex flex-col items-start gap-0.5">
						<span>Restrict to selected libraries</span>
						<span class="text-xs font-normal text-muted-foreground">Off: the device sees everything you can see.</span>
					</Label>
					<Switch id={`restrict-${device.id}`} bind:checked={draftRestricted} />
				</div>
				{#if draftRestricted}
					<div class="flex max-h-64 flex-col gap-2 overflow-y-auto rounded-lg border p-3">
						{#if librariesQuery.isPending}
							<p class="text-sm text-muted-foreground">Loading libraries…</p>
						{:else if librariesQuery.isError}
							<div class="flex items-center justify-between gap-2">
								<p class="text-sm text-destructive">{errorMessage(librariesQuery.error)}</p>
								<Button type="button" size="sm" variant="outline" onclick={() => librariesQuery.refetch()}>Retry</Button>
							</div>
						{:else if !libraries.length}
							<p class="text-sm text-muted-foreground">You have no libraries yet.</p>
						{:else}
							{#each libraries as library (library.id)}
								<div class="flex items-center gap-3">
									<Checkbox
										id={`scope-${device.id}-${library.id}`}
										checked={draftScope.includes(library.id)}
										onCheckedChange={(checked) => toggleLibrary(library.id, checked === true)}
									/>
									<Label for={`scope-${device.id}-${library.id}`} class="font-normal">
										{library.emoji ? `${library.emoji} ${library.name}` : library.name}
									</Label>
								</div>
							{/each}
						{/if}
					</div>
					{#if !draftScope.length}
						<p class="text-xs text-muted-foreground">With nothing selected the device sees no library at all.</p>
					{/if}
				{/if}
				<Dialog.Footer>
					<Button type="button" variant="ghost" onclick={() => (panel = null)}>Cancel</Button>
					<Button type="submit" disabled={setScope.isPending}>{setScope.isPending ? 'Saving…' : 'Save'}</Button>
				</Dialog.Footer>
			</form>
		{:else if panel === 'preset'}
			{@const draftEntry = TRANSFORM_PRESETS.find((candidate) => candidate.name === draftPreset)}
			<form class="flex flex-col gap-5" onsubmit={savePreset}>
				<Dialog.Header>
					<Dialog.Title>Comic preset</Dialog.Title>
					<Dialog.Description>How comics are resized and encoded for {device.name}. The stored files never change.</Dialog.Description>
				</Dialog.Header>
				<div class="flex flex-col gap-2">
					<Label for={`preset-${device.id}`}>Preset</Label>
					<Select.Root type="single" bind:value={draftPreset}>
						<Select.Trigger id={`preset-${device.id}`} class="w-full">
							{draftEntry?.label ?? (draftPreset === NO_PRESET ? 'Server default' : `Custom (${draftPreset})`)}
						</Select.Trigger>
						<Select.Content>
							<Select.Item value={NO_PRESET} label="Server default" />
							<Select.Group>
								<Select.Label>Presets</Select.Label>
								{#each TRANSFORM_PRESETS as candidate (candidate.name)}
									<Select.Item value={candidate.name} label={candidate.label} />
								{/each}
							</Select.Group>
						</Select.Content>
					</Select.Root>
					<p class="text-xs text-muted-foreground">Audiobooks: {audioDeliveryOf(draftPreset === NO_PRESET ? null : { preset: draftPreset })}. Only Phone + Opus transcodes; every other preset serves the stored file.</p>
				</div>
				<Dialog.Footer>
					<Button type="button" variant="ghost" onclick={() => (panel = null)}>Cancel</Button>
					<Button type="submit" disabled={setPreset.isPending || draftPreset === preset}>{setPreset.isPending ? 'Saving…' : 'Save'}</Button>
				</Dialog.Footer>
			</form>
		{/if}
	</Dialog.Content>
</Dialog.Root>

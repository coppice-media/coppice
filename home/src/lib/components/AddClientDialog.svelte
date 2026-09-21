<script lang="ts">
	import { browser } from '$app/environment'
	import { untrack } from 'svelte'
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query'
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left'
	import CheckIcon from '@lucide/svelte/icons/check'
	import DownloadIcon from '@lucide/svelte/icons/download'
	import { toast } from 'svelte-sonner'
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert'
	import { Button } from '@stump/ui/components/ui/button'
	import { Checkbox } from '@stump/ui/components/ui/checkbox'
	import * as Dialog from '@stump/ui/components/ui/dialog'
	import { Input } from '@stump/ui/components/ui/input'
	import { Label } from '@stump/ui/components/ui/label'
	import * as Stepper from '@stump/ui/components/ui/stepper'
	import { Switch } from '@stump/ui/components/ui/switch'
	import { request } from '@stump/ui/graphql/client'
	import { errorMessage } from '@stump/ui/utils/errors.js'
	import { cn } from '@stump/ui/utils.js'
	import {
		ConsoleLibraryOptionsDocument,
		CreateDeviceDocument,
		SetDeviceLibraryScopeDocument,
		type DeviceKind
	} from '$lib/graphql/generated/graphql'
	import {
		CLIENT_CATALOG,
		DEVICE_KIND_LABELS,
		PERMISSION_LABELS,
		clientCapabilityAvailability,
		clientProfileForVariant,
		clientVariantForId,
		clientVariants,
		defaultDeviceName,
		missingPermissionsForClient,
		type CatalogDeviceKind,
		type ClientAppVariant,
		type ClientCatalogEntry,
		type DeviceCapabilityDescriptor,
		type IssuedCredential
	} from '$lib/devices'
	import { downloadCoppicePlugin } from '$lib/coppice-plugin'
	import { getHomeSession } from '$lib/session.svelte'
	import ClientPicker from './ClientPicker.svelte'
	import CredentialReveal from './CredentialReveal.svelte'

	let {
		open = $bindable(false),
		capabilities = null,
		hideDisabled = false
	}: {
		open?: boolean
		capabilities?: readonly DeviceCapabilityDescriptor[] | null
		hideDisabled?: boolean
	} = $props()

	const session = getHomeSession()
	const queryClient = useQueryClient()

	const STEPS = [
		{ step: 1, title: 'Choose', description: 'What you are connecting' },
		{ step: 2, title: 'Name or pair', description: 'How it connects' },
		{ step: 3, title: 'Connect', description: 'Secret and endpoints' }
	]

	let step = $state(1)
	let selectedCatalogId = $state<string | null>('liseur')
	let selectedVariantId = $state<string | null>('liseur-native')
	let pickerReset = $state(0)
	let kind = $state<CatalogDeviceKind>('LISEUR')
	let name = $state('')
	let issued = $state<IssuedCredential | null>(null)
	let saved = $state(false)
	let createError = $state<string | null>(null)
	let scopeError = $state<string | null>(null)
	let scopeRestricted = $state(false)
	let scopeIds = $state<string[]>([])
	let pluginDownloadPending = $state(false)
	let pluginDownloadError = $state<string | null>(null)

	const chosen = $derived(
		selectedCatalogId ? (CLIENT_CATALOG.find((entry) => entry.id === selectedCatalogId) ?? null) : null
	)
	const chosenVariants = $derived(chosen ? clientVariants(chosen) : [])
	const chosenVariant = $derived(
		chosen ? clientVariantForId(chosen, selectedVariantId) : null
	)
	const chosenProfile = $derived(
		chosen && chosenVariant ? clientProfileForVariant(chosen, chosenVariant) : null
	)
	const isPairing = $derived(chosenVariant?.connection === 'pairing')
	const serverAvailability = $derived(
		chosenVariant
			? clientCapabilityAvailability(chosenVariant.kind, capabilities)
			: { available: false, reason: 'Choose an integration.' }
	)
	const missingPermissions = $derived(
		chosen && chosenVariant && !isPairing
			? missingPermissionsForClient(chosen, session.user, chosenVariant.kind)
			: []
	)
	const canCreate = $derived(
		Boolean(chosen && chosenVariant) &&
			serverAvailability.available &&
			(isPairing || missingPermissions.length === 0)
	)
	const librariesQuery = createQuery(() => ({
		queryKey: ['libraryOptions'],
		queryFn: () => request(ConsoleLibraryOptionsDocument, {}),
		enabled: browser && open && step === 2 && scopeRestricted && !isPairing
	}))
	const libraries = $derived(librariesQuery.data?.libraries.nodes ?? [])

	const suggestedName = $derived(
		session.user && chosenVariant
			? defaultDeviceName(session.user.username, chosenVariant.kind)
			: chosenVariant
				? DEVICE_KIND_LABELS[chosenVariant.kind]
				: DEVICE_KIND_LABELS[kind]
	)

	const setScope = createMutation(() => ({
		mutationFn: ({ deviceId, libraryIds }: { deviceId: string; libraryIds: string[] }) =>
			request(SetDeviceLibraryScopeDocument, { id: deviceId, libraryIds }),
		onSuccess: (result) => {
			if (issued) issued = { ...issued, device: result.setDeviceLibraryScope }
			scopeError = null
			void queryClient.invalidateQueries({ queryKey: ['devices'] })
		},
		onError: (error) => {
			const message = errorMessage(error)
			scopeError = message
			toast.error(message)
		}
	}))

	const createDevice = createMutation(() => ({
		mutationFn: () => {
			const trimmed = name.trim()
			const selectedKind = chosenVariant?.kind ?? kind
			return request(CreateDeviceDocument, {
				kind: selectedKind as DeviceKind,
				name: trimmed && trimmed !== suggestedName ? trimmed : null
			})
		},
		onSuccess: (result) => {
			issued = result.createDevice
			saved = false
			createError = null
			scopeError = null
			step = 3
			if (scopeRestricted) {
				setScope.mutate({
					deviceId: result.createDevice.device.id,
					libraryIds: [...scopeIds]
				})
			} else {
				void queryClient.invalidateQueries({ queryKey: ['devices'] })
			}
		},
		onError: (error) => {
			const message = errorMessage(error)
			createError = message
			toast.error(message)
		}
	}))

	function selectCatalog(entry: ClientCatalogEntry | null, variant: ClientAppVariant | null = null): void {
		selectedCatalogId = entry?.id ?? null
		selectedVariantId = variant?.id ?? null
		createError = null
		pluginDownloadError = null
		if (!entry || !variant) {
			name = ''
			return
		}
		kind = variant.kind
		name = session.user ? defaultDeviceName(session.user.username, variant.kind) : DEVICE_KIND_LABELS[variant.kind]
	}
	function variantAvailability(entry: ClientCatalogEntry, variant: ClientAppVariant): {
		available: boolean
		reason: string | null
	} {
		const server = clientCapabilityAvailability(variant.kind, capabilities)
		const missing =
			variant.connection === 'pairing' ? [] : missingPermissionsForClient(entry, session.user, variant.kind)
		if (!server.available) return server
		if (missing.length) {
			return {
				available: false,
				reason: `Missing: ${missing.map((permission) => PERMISSION_LABELS[permission] ?? permission).join(', ')}.`
			}
		}
		return { available: true, reason: null }
	}

	function selectVariant(variant: ClientAppVariant): void {
		if (!chosen) return
		if (!variantAvailability(chosen, variant).available) return
		selectCatalog(chosen, variant)
	}

	function continueToName(): void {
		if (!chosen || !chosenVariant) {
			createError = 'Choose a client before continuing.'
			return
		}
		if (!serverAvailability.available) {
			createError = serverAvailability.reason ?? 'This integration is disabled on the server.'
			return
		}
		if (!canCreate) {
			createError = missingPermissions.length
				? `This account is missing: ${missingPermissions
						.map((permission) => PERMISSION_LABELS[permission] ?? permission)
						.join(', ')}.`
				: 'This integration cannot be configured by this account.'
			return
		}
		createError = null
		if (!isPairing) name = suggestedName
		step = 2
	}
	function submit(event: SubmitEvent): void {
		event.preventDefault()
		createError = null
		if (chosenVariant?.connection !== 'credential') return
		if (!serverAvailability.available) {
			createError = serverAvailability.reason ?? 'This integration is disabled on the server.'
			step = 1
			return
		}
		if (!canCreate) {
			createError = missingPermissions.length
				? `This account is missing: ${missingPermissions
						.map((permission) => PERMISSION_LABELS[permission] ?? permission)
						.join(', ')}.`
				: 'This integration cannot be configured by this account.'
			step = 1
			return
		}
		if (!name.trim()) {
			createError = 'Give this client a name before creating its credential.'
			return
		}
		createDevice.mutate()
	}

	function retryScope(): void {
		if (!issued || !scopeRestricted) return
		scopeError = null
		setScope.mutate({ deviceId: issued.device.id, libraryIds: [...scopeIds] })
	}

	function toggleLibrary(id: string, checked: boolean): void {
		scopeIds = checked ? [...scopeIds, id] : scopeIds.filter((entry) => entry !== id)
	}

	async function downloadPlugin(): Promise<void> {
		if (pluginDownloadPending) return
		pluginDownloadPending = true
		pluginDownloadError = null
		try {
			await downloadCoppicePlugin()
			toast.success('coppice.koplugin.zip downloaded')
		} catch (error) {
			pluginDownloadError = error instanceof Error ? error.message : 'The Coppice plugin archive could not be downloaded.'
			toast.error(pluginDownloadError)
		} finally {
			pluginDownloadPending = false
		}
	}

	function reset(): void {
		step = 1
		selectedCatalogId = 'liseur'
		selectedVariantId = 'liseur-native'
		pickerReset += 1
		kind = 'LISEUR'
		name = ''
		issued = null
		saved = false
		createError = null
		scopeError = null
		scopeRestricted = false
		scopeIds = []
		pluginDownloadPending = false
		pluginDownloadError = null
	}

	$effect(() => {
		if (!open) untrack(reset)
	})
</script>

<Dialog.Root bind:open>
	<Dialog.Content
		class="flex max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] flex-col gap-4 overflow-hidden sm:max-w-5xl"
		showCloseButton={step !== 3}
		escapeKeydownBehavior={step === 3 ? 'ignore' : 'close'}
		interactOutsideBehavior={step === 3 ? 'ignore' : 'close'}
	>
		<Dialog.Header>
			<Dialog.Title>
				{#if step === 3 && issued}
					{issued.device.name} is connected
				{:else}
					Add a client
				{/if}
			</Dialog.Title>
			<Dialog.Description>
				{#if step === 1}
					Choose a reader or integration. Credentials inherit your permissions and can only narrow library access.
				{:else if step === 2 && chosen && chosenVariant}
					{#if isPairing}
						Install and pair {chosenVariant.label}; this path issues its credentials on-device after approval.
					{:else}
						Name the {chosenVariant.label.toLowerCase()} and choose which of your libraries it may see.
					{/if}
				{:else}
					Copy the secret first, then point the {chosen?.title ?? DEVICE_KIND_LABELS[kind]} at the endpoint below.
				{/if}
			</Dialog.Description>
		</Dialog.Header>

		<Stepper.Root current={step} orientation="horizontal">
			{#each STEPS as item (item.step)}
				<Stepper.Item step={item.step} title={item.title} description={item.description} />
			{/each}
		</Stepper.Root>

		{#if step === 1}
			<div class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-scroll overscroll-contain pr-2 [scrollbar-gutter:stable]">
				<p class="hidden rounded-lg border border-dashed bg-muted/30 px-3 py-2 text-xs text-muted-foreground sm:block">
					<strong class="font-medium text-foreground">Permission-safe by design.</strong>
					Raw API keys inherit this account’s permissions; library scope can only narrow visibility.
				</p>
				{#key pickerReset}
					<ClientPicker
						bind:selectedId={selectedCatalogId}
						bind:selectedVariantId
						user={session.user}
						{capabilities}
						{hideDisabled}
						onselect={selectCatalog}
					/>
				{/key}
			</div>
			{#if createError}
				<Alert variant="destructive">
					<AlertTitle>Choose an available client</AlertTitle>
					<AlertDescription>{createError}</AlertDescription>
				</Alert>
			{/if}
			<p class="text-xs text-muted-foreground">Scroll the catalog for more clients and paging.</p>
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)}>Cancel</Button>
				<Button type="button" onclick={continueToName} disabled={!canCreate}>Continue</Button>
			</Dialog.Footer>
		{:else if step === 2 && chosen && chosenVariant}
			{@const Icon = chosenVariant.icon ?? chosen.icon}
			<div class="flex min-h-0 flex-1 flex-col gap-3">
				<div class="flex items-center gap-3 rounded-xl border bg-muted/40 p-3 text-sm">
					<span class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-background text-muted-foreground">
						<Icon class="size-4" aria-hidden="true" />
					</span>
					<div class="min-w-0 flex-1">
						<div class="flex flex-wrap items-center gap-1.5 font-medium">
							<span>{chosen.title}</span>
							<span class="rounded-md border bg-background px-1.5 py-0.5 text-[10px] font-normal text-muted-foreground">
								{chosenVariant.label}
							</span>
						</div>
						<div class="line-clamp-2 text-xs text-muted-foreground">
							{chosenVariant.protocol} · {chosenVariant.setup}
						</div>
					</div>
					<Button type="button" variant="ghost" size="sm" onclick={() => (step = 1)}>Change</Button>
				</div>
				<fieldset class="flex min-w-0 flex-col gap-2 rounded-xl border p-3">
					<legend class="px-1 text-sm font-medium">Client and protocol</legend>
					<p class="text-xs text-muted-foreground">
						Choose another compatible app or protocol for {chosen.title} without returning to the client list.
					</p>
					<div class="grid min-w-0 gap-2 sm:grid-cols-2">
						{#each chosenVariants as candidate (candidate.id)}
							{@const CandidateIcon = candidate.icon ?? chosen.icon}
							{@const candidateStatus = variantAvailability(chosen, candidate)}
							{@const candidateSelected = selectedVariantId === candidate.id}
							<Button
								type="button"
								variant={candidateSelected ? 'secondary' : 'outline'}
								class="h-auto min-w-0 justify-start gap-2 p-2 text-left"
								disabled={!candidateStatus.available}
								aria-pressed={candidateSelected}
								aria-label={`${candidate.label} via ${candidate.protocol}${candidateSelected ? ', current selection' : ''}`}
								title={!candidateStatus.available ? candidateStatus.reason ?? undefined : undefined}
								onclick={() => selectVariant(candidate)}
							>
								<CandidateIcon class="size-4 shrink-0" aria-hidden="true" />
								<span class="min-w-0 flex-1">
									<span class="block truncate text-xs font-medium">{candidate.label}</span>
									<span class="block truncate text-[11px] text-muted-foreground">{candidate.protocol}</span>
									{#if !candidateStatus.available}
										<span class="block truncate text-[10px] text-destructive">{candidateStatus.reason}</span>
									{/if}
								</span>
								{#if candidateSelected}
									<CheckIcon class="size-4 shrink-0 text-primary" aria-hidden="true" />
									<span class="sr-only">Current selection</span>
								{/if}
							</Button>
						{/each}
					</div>
				</fieldset>

				{#if isPairing}
					<section class="flex flex-col gap-3 rounded-xl border bg-muted/20 p-3" aria-labelledby="pairing-setup-title">
						<div>
							<h3 id="pairing-setup-title" class="text-sm font-medium">Install and pair on-device</h3>
							<p class="mt-0.5 text-xs text-muted-foreground">{chosenVariant.setup}</p>
						</div>
						<ol class="flex list-decimal flex-col gap-1 pl-5 text-xs text-muted-foreground">
							{#each chosenVariant.setupProfile.steps as setupStep (setupStep)}
								<li>{setupStep}</li>
							{/each}
						</ol>
						{#if chosenVariant.setupProfile.download === 'coppice'}
							<div class="flex flex-col gap-2 rounded-lg border border-dashed bg-background/60 p-3">
								<div>
									<h4 class="text-sm font-medium">Download coppice.koplugin</h4>
									<p class="mt-0.5 text-xs text-muted-foreground">
										The archive is built from the generic plugin and contains only this server origin and non-secret defaults.
									</p>
								</div>
								<Button type="button" variant="outline" size="sm" class="w-fit" onclick={downloadPlugin} disabled={pluginDownloadPending}>
									<DownloadIcon data-icon="inline-start" />
									{pluginDownloadPending ? 'Preparing archive…' : 'Download coppice.koplugin.zip'}
								</Button>
								{#if pluginDownloadError}
									<p class="text-xs text-destructive" role="alert">{pluginDownloadError}</p>
								{/if}
							</div>
						{/if}
						<p class="text-xs font-medium text-muted-foreground">
							After the device displays its six-digit code, approve it from Pending pairings in Home. No account password or reusable secret is entered here.
						</p>
						{#if String(chosenVariant.kind) === 'CROSSPOINT'}
							<p class="text-xs font-medium text-muted-foreground">
								After approval, the device receives the keyed sync URL under <code>/koreader/&lt;key&gt;</code>; any rich API is mounted only below <code>/koreader/&lt;key&gt;/api/v1</code>. The current physical firmware does not use that rich API at a custom URL.
							</p>
						{/if}
						{#if chosenVariant.caveat}
							<p class="text-xs font-medium text-muted-foreground">{chosenVariant.caveat}</p>
						{/if}
						{#if chosenVariant.setupProfile.docs}
							<a
								href={chosenVariant.setupProfile.docs.href}
								target="_blank"
								rel="noreferrer"
								class="inline-flex w-fit items-center text-xs text-primary underline-offset-4 hover:underline"
							>
								{chosenVariant.setupProfile.docs.label}
							</a>
						{/if}
					</section>
					<Dialog.Footer>
						<Button type="button" variant="ghost" onclick={() => (step = 1)}>
							<ArrowLeftIcon data-icon="inline-start" />
							Back to choices
						</Button>
						<Button type="button" onclick={() => (open = false)}>Back to Devices</Button>
					</Dialog.Footer>
				{:else}
					<form class="flex min-h-0 flex-col gap-4" onsubmit={submit}>
						<div class="flex flex-col gap-2">
							<Label for="device-name">Name</Label>
							<Input
								id="device-name"
								bind:value={name}
								maxlength={100}
								autocomplete="off"
								required
								autofocus
								aria-describedby="device-name-help"
							/>
							<p id="device-name-help" class="text-xs text-muted-foreground">
								Shown on this page and in reading stats. Keep the suggestion or name it after the device.
							</p>
						</div>
						<div class="flex flex-col gap-3 rounded-xl border p-3">
							<div class="flex items-start justify-between gap-4">
								<Label for="device-scope" class="flex flex-col items-start gap-0.5">
									<span>Limit visible libraries</span>
									<span class="text-xs font-normal text-muted-foreground">Off: this client inherits the libraries you can already see.</span>
								</Label>
								<Switch id="device-scope" bind:checked={scopeRestricted} />
							</div>
							{#if scopeRestricted}
								<div class="flex max-h-52 flex-col gap-2 overflow-y-auto rounded-lg border p-3">
									{#if librariesQuery.isPending}
										<p class="text-sm text-muted-foreground">Loading libraries…</p>
									{:else if librariesQuery.isError}
										<div class="flex items-center justify-between gap-2">
											<p class="text-sm text-destructive">{errorMessage(librariesQuery.error)}</p>
											<Button type="button" size="sm" variant="outline" onclick={() => librariesQuery.refetch()}>Retry</Button>
										</div>
									{:else if !libraries.length}
										<p class="text-sm text-muted-foreground">You have no libraries to select.</p>
									{:else}
										{#each libraries as library (library.id)}
											<div class="flex items-center gap-3">
												<Checkbox
													id={`new-device-scope-${library.id}`}
													checked={scopeIds.includes(library.id)}
													onCheckedChange={(checked) => toggleLibrary(library.id, checked === true)}
												/>
												<Label for={`new-device-scope-${library.id}`} class="font-normal">
													{library.emoji ? `${library.emoji} ${library.name}` : library.name}
												</Label>
											</div>
										{/each}
									{/if}
								</div>
								<p class="text-xs text-muted-foreground">Selecting none intentionally gives this client no library access.</p>
							{/if}
						</div>
						{#if createError}
							<Alert variant="destructive">
								<AlertTitle>Could not create the client</AlertTitle>
								<AlertDescription>{createError}</AlertDescription>
							</Alert>
						{/if}
						<Dialog.Footer>
							<Button type="button" variant="ghost" onclick={() => (step = 1)}>
								<ArrowLeftIcon data-icon="inline-start" />
								Back
							</Button>
							<Button
								type="submit"
								disabled={createDevice.isPending || (scopeRestricted && (librariesQuery.isPending || librariesQuery.isError))}
							>
								{createDevice.isPending ? 'Creating…' : 'Create credential'}
							</Button>
						</Dialog.Footer>
					</form>
				{/if}
			</div>
		{:else if issued}
			<div class="min-h-0 flex-1 overflow-y-auto pr-1">
				{#if scopeRestricted && setScope.isPending}
					<Alert>
						<AlertTitle>Applying library scope…</AlertTitle>
						<AlertDescription>The credential is ready, but this client is not finished until its library restriction is saved.</AlertDescription>
					</Alert>
				{:else if scopeError}
					<Alert variant="destructive">
						<AlertTitle>Library scope was not saved</AlertTitle>
						<AlertDescription>{scopeError} The client remains unconfigured until the restriction is applied.</AlertDescription>
						<Button type="button" variant="outline" class="mt-3" onclick={retryScope}>Retry scope</Button>
					</Alert>
				{/if}
				<div class={cn(scopeError || setScope.isPending ? 'mt-4' : undefined)}>
					<CredentialReveal {issued} profile={chosenProfile ?? undefined} bind:saved />
				</div>
			</div>
			<Dialog.Footer>
				<Button
					type="button"
					onclick={() => (open = false)}
					disabled={!saved || (scopeRestricted && (setScope.isPending || scopeError !== null))}
				>
					Done
				</Button>
			</Dialog.Footer>
		{/if}
	</Dialog.Content>
</Dialog.Root>

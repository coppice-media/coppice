<script lang="ts">
	import { browser } from '$app/environment'
	import CheckIcon from '@lucide/svelte/icons/check'
	import CopyIcon from '@lucide/svelte/icons/copy'
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link'
	import KeyRoundIcon from '@lucide/svelte/icons/key-round'
	import { Badge } from '@stump/ui/components/ui/badge'
	import { Button } from '@stump/ui/components/ui/button'
	import { Checkbox } from '@stump/ui/components/ui/checkbox'
	import { Label } from '@stump/ui/components/ui/label'
	import { copyText } from '$lib/clipboard'
	import {
		DEVICE_KIND_LABELS,
		PROTOCOL_LABELS,
		SETUP_HINTS,
		libraryScopeSummary,
		protocolForKind,
		type ClientCatalogEntry,
		type IssuedCredential
	} from '$lib/devices'
	import EndpointCard from './EndpointCard.svelte'

	let {
		issued,
		profile,
		saved = $bindable(false)
	}: {
		issued: IssuedCredential
		/** Presentation profile used by the add flow; rotations use kind defaults. */
		profile?: ClientCatalogEntry
		/** The user's word that the secret is stored somewhere; the dialog's Done waits for it. */
		saved?: boolean
	} = $props()

	const hint = $derived(profile?.setupProfile ?? SETUP_HINTS[issued.device.kind])
	const displayTitle = $derived(profile?.title ?? DEVICE_KIND_LABELS[issued.device.kind])
	const displayDescription = $derived(profile?.description)
	const protocol = $derived(profile?.protocol ?? PROTOCOL_LABELS[protocolForKind(issued.device.kind)])
	const savedId = $derived(`saved-${issued.device.id}`)
	const scopeLabel = $derived(libraryScopeSummary(issued.device.libraryScope))
	const komfUrl = $derived(
		issued.device.kind === 'KOMELIA' && browser
			? `http://${window.location.host}/?apiKey=${encodeURIComponent(issued.credential.secret)}`
			: null
	)
	let copied = $state(false)
	let komfCopied = $state(false)

	async function copySecret(): Promise<void> {
		if (!(await copyText(issued.credential.secret, 'secret'))) return
		copied = true
		setTimeout(() => (copied = false), 2000)
	}

	async function copyKomfUrl(): Promise<void> {
		if (!komfUrl || !(await copyText(komfUrl, 'Komf URL'))) return
		komfCopied = true
		setTimeout(() => (komfCopied = false), 2000)
	}
</script>

<div class="flex flex-col gap-5">
	<section class="rounded-xl border bg-card p-4 sm:p-5" aria-labelledby={`${savedId}-secret`}>
		<div class="flex items-start gap-3">
			<span class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
				<KeyRoundIcon class="size-4" />
			</span>
			<div class="min-w-0 flex-1">
				<h3 id={`${savedId}-secret`} class="text-sm font-medium">Secret for {issued.device.name}</h3>
				{#if displayDescription}
					<p class="mt-0.5 text-xs text-muted-foreground">
						{displayTitle}: {displayDescription}
					</p>
				{/if}
				<p class="mt-1 text-xs text-muted-foreground">
					You will not see this again. Coppice keeps only a hash of it; if it is lost, rotate the
					credential for a new one.
				</p>
				<div class="mt-3 flex flex-wrap gap-1.5">
					<Badge variant="outline">{protocol}</Badge>
					<Badge variant="secondary">Scope: {scopeLabel}</Badge>
					<Badge variant="secondary">Credential stored as a hash</Badge>
				</div>
			</div>
		</div>
		<div class="mt-4 flex flex-col gap-2 sm:flex-row sm:items-stretch">
			<code class="min-w-0 flex-1 rounded-lg border bg-muted/50 px-3 py-2 font-mono text-sm break-all select-all">
				{issued.credential.secret}
			</code>
			<Button onclick={copySecret} class="sm:shrink-0" aria-live="polite">
				{#if copied}
					<CheckIcon data-icon="inline-start" />
					Copied
				{:else}
					<CopyIcon data-icon="inline-start" />
					Copy secret
				{/if}
			</Button>
		</div>
	</section>

	{#if issued.endpoints.length}
		<section class="flex flex-col gap-3" aria-label="Where to point the client">
			<h3 class="text-sm font-medium">Where to point the {displayTitle}</h3>
			{#each issued.endpoints as endpoint (endpoint.label + endpoint.url)}
				<EndpointCard {endpoint} />
			{/each}
		</section>
	{:else}
		<p class="text-sm text-muted-foreground">
			This client kind has no fixed endpoint; use the secret as an API key against this server's origin.
		</p>
	{/if}
	{#if komfUrl}
		<section class="flex flex-col gap-2 rounded-xl border bg-muted/20 p-3" aria-labelledby={`${savedId}-komf-url`}>
			<div>
				<h3 id={`${savedId}-komf-url`} class="text-sm font-medium">Komf URL</h3>
				<p class="text-xs text-muted-foreground">
					Use this URL in Komelia for Komf metadata editing. It includes the device API key.
				</p>
			</div>
			<div class="flex flex-col gap-2 sm:flex-row">
				<code class="min-w-0 flex-1 rounded-lg border bg-background px-3 py-2 font-mono text-sm break-all select-all">{komfUrl}</code>
				<Button type="button" variant="outline" onclick={copyKomfUrl} class="sm:shrink-0" aria-live="polite">
					{#if komfCopied}
						<CheckIcon data-icon="inline-start" />
						Copied
					{:else}
						<CopyIcon data-icon="inline-start" />
						Copy Komf URL
					{/if}
				</Button>
			</div>
		</section>
	{/if}

	{#if hint}
		<section class="flex flex-col gap-2 text-sm" aria-label="Setup steps">
			<h3 class="font-medium">On the device</h3>
			<ol class="flex list-decimal flex-col gap-1 pl-5 text-muted-foreground">
				{#each hint.steps as step (step)}
					<li>{step}</li>
				{/each}
			</ol>
			{#if hint.docs}
				<a
					href={hint.docs.href}
					target="_blank"
					rel="noreferrer"
					class="inline-flex w-fit items-center gap-1 text-primary underline-offset-4 hover:underline"
				>
					{hint.docs.label}
					<ExternalLinkIcon class="size-3.5" aria-hidden="true" />
				</a>
			{/if}
		</section>
	{/if}

	<div class="flex items-start gap-3 rounded-xl border border-dashed p-3">
		<Checkbox id={savedId} bind:checked={saved} class="mt-0.5" />
		<Label for={savedId} class="text-sm leading-snug font-normal">
			I have copied the secret somewhere safe and understand it is not shown again.
		</Label>
	</div>
</div>

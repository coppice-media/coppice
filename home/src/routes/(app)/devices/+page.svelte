<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import ServerCogIcon from '@lucide/svelte/icons/server-cog';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Switch } from '@stump/ui/components/ui/switch';
	import {
		Empty,
		EmptyContent,
		EmptyDescription,
		EmptyHeader,
		EmptyTitle
	} from '@stump/ui/components/ui/empty';
	import { request, subscribe } from '@stump/ui/graphql/client';
	import { DeviceCapabilitiesDocument, DeviceSeenDocument, DevicesDocument } from '$lib/graphql/generated/graphql';
	import {
		HIDE_DISABLED_CLIENTS_STORAGE_KEY,
		type DeviceCapabilityDescriptor
	} from '$lib/devices';
	import AddClientDialog from '$lib/components/AddClientDialog.svelte';
	import DeviceCard from '$lib/components/DeviceCard.svelte';
	import PendingPairings from '$lib/components/PendingPairings.svelte';

	const capabilitiesQuery = createQuery(() => ({
		queryKey: ['device-capabilities'],
		queryFn: () => request(DeviceCapabilitiesDocument, {}),
		enabled: browser
	}))
	const capabilities = $derived(
		(capabilitiesQuery.data?.deviceCapabilities ?? []) as DeviceCapabilityDescriptor[]
	)
	const availableCapabilityCount = $derived(capabilities.filter((capability) => capability.available).length)
	const capabilitiesReady = $derived(
		!capabilitiesQuery.isPending && !capabilitiesQuery.isError && capabilitiesQuery.data !== undefined
	)
	const canAddClient = $derived(capabilitiesReady && availableCapabilityCount > 0)

	let hideDisabled = $state(false)
	let preferenceLoaded = $state(false)
	$effect(() => {
		if (!browser || preferenceLoaded) return
		hideDisabled = localStorage.getItem(HIDE_DISABLED_CLIENTS_STORAGE_KEY) === 'true'
		preferenceLoaded = true
	})
	$effect(() => {
		if (browser && preferenceLoaded) {
			localStorage.setItem(HIDE_DISABLED_CLIENTS_STORAGE_KEY, String(hideDisabled))
		}
	})

	const queryClient = useQueryClient();
	const devicesQuery = createQuery(() => ({
		queryKey: ['devices'],
		queryFn: () => request(DevicesDocument, {}),
		enabled: browser
	}));

	// Active devices first, then revoked; newest registration on top within each group.
	const devices = $derived(
		[...(devicesQuery.data?.devices ?? [])].sort((a, b) => {
			const revokedA = a.revokedAt ? 1 : 0;
			const revokedB = b.revokedAt ? 1 : 0;
			if (revokedA !== revokedB) return revokedA - revokedB;
			return b.createdAt.localeCompare(a.createdAt);
		})
	);

	let addOpen = $state(false);
	let liveError = $state<string | null>(null);
	// Ticks once a minute so "last seen 3m ago" stays honest without refetching.
	let now = $state(new Date());

	$effect(() => {
		const timer = setInterval(() => (now = new Date()), 60_000);
		return () => clearInterval(timer);
	});

	// A device authenticating anywhere updates its last-seen timestamp; refetch
	// the list so the card reflects it within a round-trip.
	$effect(() => {
		return subscribe(DeviceSeenDocument, {}, {
			next: (result) => {
				if (!result.data?.deviceSeen) return;
				now = new Date();
				void queryClient.invalidateQueries({ queryKey: ['devices'] });
			},
			error: () => {
				liveError = 'Live device activity is unavailable; last-seen times update on reload.';
			},
			complete: () => undefined
		});
	});
</script>

<svelte:head>
	<title>Devices · Coppice</title>
</svelte:head>

<div class="flex flex-col gap-8">
	<div class="flex flex-wrap items-center gap-3">
		<div class="mr-auto">
			<h1 class="text-2xl font-semibold tracking-tight">Devices</h1>
			<p class="text-sm text-muted-foreground">
				Readers and integrations signed in with a credential of their own. CrossPoint adds a
				confirmed private-LAN delivery target; credentials stay within your account permissions
				and a device scope can only narrow library access.
			</p>
		</div>
		<Button disabled={!canAddClient} title={!canAddClient ? 'Server integrations are unavailable' : undefined} onclick={() => (addOpen = true)}>
			<PlusIcon data-icon="inline-start" />
			{#if capabilitiesQuery.isPending}
				Checking integrations…
			{:else}
				Add client
			{/if}
		</Button>
	</div>
	<section aria-labelledby="device-capabilities-heading" class="rounded-xl border bg-card p-4">
		<div class="flex flex-wrap items-start justify-between gap-4">
			<div class="min-w-0">
				<h2 id="device-capabilities-heading" class="text-sm font-semibold">Server integrations</h2>
				{#if capabilitiesQuery.isPending}
					<p class="mt-1 text-xs text-muted-foreground" aria-live="polite">Checking which integrations are available…</p>
				{:else if capabilitiesQuery.isError}
					<p class="mt-1 text-xs text-muted-foreground">The server capability check failed, so client setup is paused.</p>
				{:else if capabilities.length === 0}
					<p class="mt-1 text-xs text-muted-foreground">This server advertised no client integrations.</p>
				{:else}
					<p class="mt-1 text-xs text-muted-foreground">
						{availableCapabilityCount} of {capabilities.length} server integrations are available for setup.
					</p>
				{/if}
			</div>
			{#if capabilitiesReady && capabilities.length > 0}
				<div class="flex flex-wrap items-center justify-end gap-3">
					<Badge variant={availableCapabilityCount > 0 ? 'secondary' : 'outline'}>
						{availableCapabilityCount} available
					</Badge>
					<label class="flex items-center gap-2 text-xs text-muted-foreground" for="hide-disabled-integrations">
						<Switch id="hide-disabled-integrations" bind:checked={hideDisabled} />
						<span>Hide disabled integrations</span>
					</label>
					<a
						href={resolve('/components')}
						class="inline-flex items-center gap-1.5 text-xs font-medium text-primary underline-offset-4 hover:underline"
					>
						<ServerCogIcon class="size-3.5" aria-hidden="true" />
						Components
					</a>
				</div>
			{/if}
		</div>
		{#if capabilitiesReady && capabilities.length > 0}
			<p class="mt-3 text-[11px] text-muted-foreground">
				This display preference is stored in this browser only; it never removes server configuration.
			</p>
		{/if}
		{#if capabilitiesQuery.isPending}
			<div class="mt-4 grid gap-2 sm:grid-cols-3" aria-label="Loading server integrations">
				<Skeleton class="h-9 rounded-lg" />
				<Skeleton class="h-9 rounded-lg" />
				<Skeleton class="h-9 rounded-lg" />
			</div>
		{:else if capabilitiesQuery.isError}
			<Alert variant="destructive" class="mt-4">
				<AlertTitle>Unable to load server integrations</AlertTitle>
				<AlertDescription>
					{capabilitiesQuery.error instanceof Error ? capabilitiesQuery.error.message : 'Request failed.'}
				</AlertDescription>
				<Button type="button" variant="outline" class="mt-3" onclick={() => capabilitiesQuery.refetch()}>
					<RotateCcwIcon data-icon="inline-start" />
					Retry
				</Button>
			</Alert>
		{:else if capabilities.length === 0}
			<Alert class="mt-4">
				<AlertTitle>No integrations available</AlertTitle>
				<AlertDescription>
					Client setup stays disabled until the server compiles and enables at least one integration.
				</AlertDescription>
			</Alert>
		{/if}
	</section>

	{#if liveError}
		<Alert>
			<AlertTitle>Live updates paused</AlertTitle>
			<AlertDescription>{liveError}</AlertDescription>
		</Alert>
	{/if}

	<PendingPairings {now} />

	{#if devicesQuery.isPending}
		<div class="grid gap-4 md:grid-cols-2">
			<Skeleton class="h-48 rounded-xl" />
			<Skeleton class="h-48 rounded-xl" />
		</div>
	{:else if devicesQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load devices</AlertTitle>
			<AlertDescription>
				{devicesQuery.error instanceof Error ? devicesQuery.error.message : 'Request failed.'}
			</AlertDescription>
			<Button type="button" variant="outline" class="mt-3" onclick={() => devicesQuery.refetch()}>
				<RotateCcwIcon data-icon="inline-start" />
				Retry
			</Button>
		</Alert>
	{:else if devices.length === 0}
		<Empty class="rounded-xl border border-dashed bg-card">
			<EmptyHeader>
				<EmptyTitle>No devices yet</EmptyTitle>
				<EmptyDescription>
					Add a client to mint a credential for your Kobo, KOReader, CrossPoint, Liseur, Mihon,
					Komelia, or any OPDS reader.
				</EmptyDescription>
			</EmptyHeader>
			<EmptyContent>
				<Button disabled={!canAddClient} onclick={() => (addOpen = true)}>
					<PlusIcon data-icon="inline-start" />
					{canAddClient ? 'Add your first client' : 'No integrations available'}
				</Button>
			</EmptyContent>
		</Empty>
	{:else}
		<div class="grid items-stretch gap-4 md:grid-cols-2">
			{#each devices as device (device.id)}
				<DeviceCard {device} {now} />
			{/each}
		</div>
	{/if}
</div>

<AddClientDialog bind:open={addOpen} capabilities={capabilities} {hideDisabled} />

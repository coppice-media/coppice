<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import LinkIcon from '@lucide/svelte/icons/link';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import UnplugIcon from '@lucide/svelte/icons/unplug';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConnectionConnectHardcoverDocument,
		ConnectionDisconnectHardcoverDocument,
		ConnectionHardcoverDocument,
		ConnectionHardcoverLinksDocument,
		ConnectionLinkHardcoverMediaDocument,
		ConnectionSyncHardcoverNowDocument,
		ConnectionUnlinkHardcoverMediaDocument,
		ConnectionUpdateHardcoverDocument
	} from '$lib/graphql/generated/graphql';
	import { absoluteTime } from '$lib/format';

	const queryClient = useQueryClient();
	const hardcoverQuery = createQuery(() => ({
		queryKey: ['hardcover-connection'],
		queryFn: () => request(ConnectionHardcoverDocument, {}),
		enabled: browser
	}));
	const connection = $derived(hardcoverQuery.data?.hardcoverConnection ?? null);
	const linksQuery = createQuery(() => ({
		queryKey: ['hardcover-links'],
		queryFn: () => request(ConnectionHardcoverLinksDocument, { mediaId: null }),
		enabled: browser && connection?.connected === true
	}));
	const links = $derived(linksQuery.data?.hardcoverMediaLinks ?? []);

	let localMediaId = $state('');
	let remoteId = $state('');
	let linking = $state(false);
	let unlinkingMediaId = $state<string | null>(null);

	let apiToken = $state('');
	let useForMetadata = $state(true);
	let importJournals = $state(false);
	let syncProgress = $state(false);
	let loadedToggles = $state(false);
	let syncing = $state(false);
	let disconnecting = $state(false);
	let syncResult = $state<{
		status: string;
		imported: number;
		unresolved: number;
		projected: number;
		skipped: number;
		lastSyncAt: string | null;
		error: string | null;
	} | null>(null);

	$effect(() => {
		if (!connection || loadedToggles) return;
		loadedToggles = true;
		useForMetadata = connection.useForMetadata;
		importJournals = connection.importJournals;
		syncProgress = connection.syncProgress;
	});

	const connect = createMutation(() => ({
		mutationFn: () =>
			request(ConnectionConnectHardcoverDocument, {
				apiToken: apiToken.trim(),
				useForMetadata,
				importJournals,
				syncProgress
			}),
		onSuccess: () => {
			apiToken = '';
			loadedToggles = false;
			void queryClient.invalidateQueries({ queryKey: ['hardcover-connection'] });
			toast.success('Hardcover connection verified.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Hardcover could not verify this token.')
	}));

	const update = createMutation(() => ({
		mutationFn: () =>
			request(ConnectionUpdateHardcoverDocument, { useForMetadata, importJournals, syncProgress }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['hardcover-connection'] });
			toast.success('Hardcover preferences saved.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to save Hardcover preferences.')
	}));

	const disconnect = createMutation(() => ({
		mutationFn: () => request(ConnectionDisconnectHardcoverDocument, {}),
		onSuccess: () => {
			apiToken = '';
			loadedToggles = false;
			void queryClient.invalidateQueries({ queryKey: ['hardcover-connection'] });
			toast.success('Hardcover disconnected. Local data stays available.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to disconnect Hardcover.'),
		onSettled: () => (disconnecting = false)
	}));

	const sync = createMutation(() => ({
		mutationFn: () => request(ConnectionSyncHardcoverNowDocument, {}),
		onSuccess: (result) => {
			syncResult = result.syncHardcoverNow;
			void queryClient.invalidateQueries({ queryKey: ['hardcover-connection'] });
			if (result.syncHardcoverNow.error) toast.error('Hardcover sync completed with an error.');
			else toast.success('Hardcover sync finished.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to sync Hardcover.'),
		onSettled: () => (syncing = false)
	}));
	const link = createMutation(() => ({
		mutationFn: () =>
			request(ConnectionLinkHardcoverMediaDocument, {
				mediaId: localMediaId.trim(),
				remoteId: remoteId.trim()
			}),
		onSuccess: () => {
			localMediaId = '';
			remoteId = '';
			void queryClient.invalidateQueries({ queryKey: ['hardcover-links'] });
			toast.success('Hardcover link saved.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to link this book.'),
		onSettled: () => (linking = false)
	}));

	const unlink = createMutation(() => ({
		mutationFn: (mediaId: string) =>
			request(ConnectionUnlinkHardcoverMediaDocument, { mediaId }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['hardcover-links'] });
			toast.success('Hardcover link removed.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to remove this link.'),
		onSettled: () => (unlinkingMediaId = null)
	}));


	function submitConnect(event: SubmitEvent): void {
		event.preventDefault();
		if (!apiToken.trim()) return;
		connect.mutate();
	}

	function saveToggles(): void {
		update.mutate();
	}

	function submitLink(event: SubmitEvent): void {
		event.preventDefault();
		if (!localMediaId.trim() || !remoteId.trim()) return;
		linking = true;
		link.mutate();
	}

	function removeLink(mediaId: string): void {
		if (!browser || !window.confirm('Remove this explicit Hardcover link? Local data stays unchanged.')) return;
		unlinkingMediaId = mediaId;
		unlink.mutate(mediaId);
	}
	function disconnectNow(): void {
		if (!browser || !window.confirm('Disconnect Hardcover? Local metadata, quotes, and progress remain in Coppice.')) return;
		disconnecting = true;
		disconnect.mutate();
	}

	function syncNow(): void {
		syncResult = null;
		syncing = true;
		sync.mutate();
	}
</script>

<Card>
	<CardHeader>
		<div class="flex flex-wrap items-start gap-2">
			<div class="mr-auto">
				<CardTitle class="text-base">Hardcover</CardTitle>
				<CardDescription>Optional, owner-only metadata and read-only journal integration.</CardDescription>
			</div>
			{#if hardcoverQuery.isPending}
				<Badge variant="outline">Checking…</Badge>
			{:else if connection?.connected}
				<Badge><CheckCircle2Icon aria-hidden="true" />Connected</Badge>
			{:else}
				<Badge variant="outline"><CircleAlertIcon aria-hidden="true" />Not connected</Badge>
			{/if}
		</div>
	</CardHeader>
	<CardContent class="flex flex-col gap-5">
		{#if hardcoverQuery.isPending}
			<div class="flex flex-col gap-3" aria-label="Loading Hardcover connection">
				<Skeleton class="h-10 w-full rounded-lg" />
				<Skeleton class="h-24 w-full rounded-lg" />
			</div>
		{:else if hardcoverQuery.isError}
			<Alert variant="destructive">
				<AlertTitle>Unable to load Hardcover status</AlertTitle>
				<AlertDescription>
					{hardcoverQuery.error instanceof Error ? hardcoverQuery.error.message : 'Request failed.'}
				</AlertDescription>
				<Button type="button" size="sm" variant="outline" class="mt-3" onclick={() => hardcoverQuery.refetch()}>
					Retry
				</Button>
			</Alert>
		{:else if !connection?.connected}
			<form class="grid gap-4 rounded-lg border bg-muted/20 p-4" onsubmit={submitConnect}>
				<div>
					<h3 class="text-sm font-medium">Connect with a personal token</h3>
					<p class="text-xs text-muted-foreground">
						Your PAT is sent for verification and stored encrypted. It is never shown again; leave the field blank after connecting.
					</p>
				</div>
				<div class="grid gap-2">
					<Label for="hardcover-token">Hardcover personal access token</Label>
					<Input id="hardcover-token" type="password" bind:value={apiToken} autocomplete="off" placeholder="Paste token once" required />
				</div>
				<div class="flex flex-wrap items-center justify-between gap-3">
					<Label for="hardcover-metadata-new" class="flex items-center gap-2 text-sm font-normal">
						<Switch id="hardcover-metadata-new" bind:checked={useForMetadata} />
						Use Hardcover for metadata (recommended)
					</Label>
					<Button type="submit" disabled={connect.isPending || !apiToken.trim()}>
						{connect.isPending ? 'Verifying…' : 'Connect Hardcover'}
					</Button>
				</div>
			</form>
		{:else}
			<div class="grid gap-4">
				<div class="flex flex-wrap gap-2 text-sm">
					{#if connection.remoteUsername}<Badge variant="secondary">{connection.remoteUsername}</Badge>{/if}
					{#if connection.remoteUserId}<span class="text-muted-foreground">Remote account {connection.remoteUserId}</span>{/if}
					{#if connection.connectedAt}<span class="text-muted-foreground">Connected {absoluteTime(connection.connectedAt)}</span>{/if}
					{#if connection.verifiedAt}<span class="text-muted-foreground">Verified {absoluteTime(connection.verifiedAt)}</span>{/if}
					{#if connection.lastSyncAt}<span class="text-muted-foreground">Last sync {absoluteTime(connection.lastSyncAt)}</span>{/if}
				</div>
				<div class="grid gap-3 rounded-lg border p-4">
					<div class="flex flex-wrap items-center justify-between gap-3">
						<div>
							<p class="text-sm font-medium">Connection preferences</p>
							<p class="text-xs text-muted-foreground">These toggles never make local writes depend on Hardcover being online.</p>
						</div>
						<Button size="sm" disabled={update.isPending} onclick={saveToggles}>{update.isPending ? 'Saving…' : 'Save preferences'}</Button>
					</div>
					<div class="grid gap-3 sm:grid-cols-3">
						<Label for="hardcover-metadata" class="flex items-start gap-2 text-sm font-normal"><Switch id="hardcover-metadata" bind:checked={useForMetadata} /><span><span class="font-medium">Metadata lookup</span><span class="mt-0.5 block text-xs text-muted-foreground">Owner-only lookup by default.</span></span></Label>
						<Label for="hardcover-journals" class="flex items-start gap-2 text-sm font-normal"><Switch id="hardcover-journals" bind:checked={importJournals} /><span><span class="font-medium">Import journals</span><span class="mt-0.5 block text-xs text-muted-foreground">Read-only quote and journal import.</span></span></Label>
						<Label for="hardcover-progress" class="flex items-start gap-2 text-sm font-normal"><Switch id="hardcover-progress" bind:checked={syncProgress} /><span><span class="font-medium">Project progress</span><span class="mt-0.5 block text-xs text-muted-foreground">Only with trustworthy remote page identity.</span></span></Label>
					</div>
				</div>

				{#if connection.scopes.length}
					<div class="grid gap-2">
						<p class="text-sm font-medium">Granted scopes</p>
						<div class="flex flex-wrap gap-1.5">
							{#each connection.scopes as scope (scope)}<Badge variant="outline">{scope}</Badge>{/each}
						</div>
					</div>
				{:else}
					<Alert>
						<AlertTitle>No remote scopes reported</AlertTitle>
						<AlertDescription>Metadata, journals, and progress remain disabled until the token grants the required capability.</AlertDescription>
					</Alert>
				{/if}

				{#if connection.capabilities.length}
					<div class="grid gap-2">
						<p class="text-sm font-medium">Current capabilities</p>
						<div class="flex flex-wrap gap-1.5">
							{#each connection.capabilities as capability (capability)}<Badge variant="secondary">{capability}</Badge>{/each}
						</div>
					</div>
				{/if}

				{#if connection.lastError}
					<Alert variant="destructive">
						<AlertTitle>Hardcover reported an issue</AlertTitle>
						<AlertDescription>{connection.lastError}</AlertDescription>
					</Alert>
				{/if}

				<div class="grid gap-3 rounded-lg border p-4">
					<div>
						<p class="text-sm font-medium">Explicit book links</p>
						<p class="text-xs text-muted-foreground">Link a local media ID to a Hardcover book ID when automatic matching is unsafe. Removing a link never deletes local data.</p>
					</div>
					{#if linksQuery.isPending}
						<Skeleton class="h-10 w-full rounded-lg" />
					{:else if linksQuery.isError}
						<Alert variant="destructive">
							<AlertTitle>Unable to load Hardcover links</AlertTitle>
							<AlertDescription>{linksQuery.error instanceof Error ? linksQuery.error.message : 'Request failed.'}</AlertDescription>
							<Button type="button" size="sm" variant="outline" class="mt-3" onclick={() => linksQuery.refetch()}>Retry</Button>
						</Alert>
					{:else if links.length}
						<ul class="flex flex-col gap-2" aria-label="Explicit Hardcover links">
							{#each links as item (item.id)}
								<li class="flex flex-wrap items-center gap-2 rounded-md border bg-muted/20 p-2 text-xs">
									<code class="font-mono">{item.mediaId}</code>
									<span aria-hidden="true">↔</span>
									<span class="min-w-0 flex-1 truncate">{item.remoteTitle ?? item.remoteId}</span>
									<Button type="button" size="sm" variant="ghost" disabled={unlinkingMediaId === item.mediaId} onclick={() => removeLink(item.mediaId)}>
										{unlinkingMediaId === item.mediaId ? 'Removing…' : 'Remove'}
									</Button>
								</li>
							{/each}
						</ul>
					{:else}
						<p class="text-xs text-muted-foreground">No explicit links yet.</p>
					{/if}
					<form class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] sm:items-end" onsubmit={submitLink}>
						<div class="grid gap-2"><Label for="hardcover-local-media">Local media ID</Label><Input id="hardcover-local-media" bind:value={localMediaId} autocomplete="off" placeholder="Coppice media ID" required /></div>
						<div class="grid gap-2"><Label for="hardcover-remote-id">Hardcover book ID</Label><Input id="hardcover-remote-id" bind:value={remoteId} autocomplete="off" placeholder="Hardcover ID" required /></div>
						<Button type="submit" variant="outline" disabled={linking || !localMediaId.trim() || !remoteId.trim()}><LinkIcon data-icon="inline-start" />{linking ? 'Linking…' : 'Link book'}</Button>
					</form>
				</div>
			</div>
		{/if}
	</CardContent>
	{#if connection?.connected}
		<CardFooter class="flex flex-wrap gap-2">
			<Button variant="outline" disabled={syncing} onclick={syncNow}>
				<RefreshCwIcon data-icon="inline-start" />
				{syncing ? 'Syncing…' : 'Sync now'}
			</Button>
			<Button variant="ghost" disabled={disconnecting} onclick={disconnectNow}>
				<UnplugIcon data-icon="inline-start" />
				{disconnecting ? 'Disconnecting…' : 'Disconnect'}
			</Button>
		</CardFooter>
	{/if}
</Card>

{#if syncResult}
	<Card>
		<CardHeader>
			<CardTitle class="text-base">Latest Hardcover sync</CardTitle>
			<CardDescription>{syncResult.lastSyncAt ? absoluteTime(syncResult.lastSyncAt) : syncResult.status}</CardDescription>
		</CardHeader>
		<CardContent class="grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
			<div><span class="block text-xs text-muted-foreground">Imported</span><span class="font-medium tabular-nums">{syncResult.imported}</span></div>
			<div><span class="block text-xs text-muted-foreground">Unresolved</span><span class="font-medium tabular-nums">{syncResult.unresolved}</span></div>
			<div><span class="block text-xs text-muted-foreground">Projected</span><span class="font-medium tabular-nums">{syncResult.projected}</span></div>
			<div><span class="block text-xs text-muted-foreground">Skipped</span><span class="font-medium tabular-nums">{syncResult.skipped}</span></div>
		</CardContent>
		{#if syncResult.error}
			<CardFooter><Alert variant="destructive"><AlertTitle>Sync warning</AlertTitle><AlertDescription>{syncResult.error}</AlertDescription></Alert></CardFooter>
		{/if}
	</Card>
{/if}

<div class="flex items-start gap-2 rounded-lg border border-dashed bg-muted/30 p-3 text-xs text-muted-foreground">
	<LinkIcon class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
	<p>Hardcover links are explicit and reversible. Coppice imports remote quotes and journals only when an exact or unique locator is available; unresolved items keep their provenance instead of being silently rewritten.</p>
</div>

<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import ServerIcon from '@lucide/svelte/icons/server';
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { request } from '@stump/ui/graphql/client';
	import {
		ConnectionAdminBoundaryDocument,
		ConnectionAdminEmailersDocument,
		ConnectionCreateAdminEmailerDocument,
		ConnectionSmtpDocument,
		ConnectionTestAdminEmailerDocument,
		ConnectionUpdateAdminEmailerDocument
	} from '$lib/graphql/generated/graphql';
	import { absoluteTime } from '$lib/format';

	const queryClient = useQueryClient();
	const emailersQuery = createQuery(() => ({
		queryKey: ['admin-smtp'],
		queryFn: () => request(ConnectionAdminEmailersDocument, {}),
		enabled: browser
	}));
	const smtpQuery = createQuery(() => ({
		queryKey: ['admin-smtp-status'],
		queryFn: () => request(ConnectionSmtpDocument, {}),
		enabled: browser
	}));
	const boundaryQuery = createQuery(() => ({
		queryKey: ['admin-connection-boundary'],
		queryFn: () => request(ConnectionAdminBoundaryDocument, {}),
		enabled: browser
	}));
	const smtpStatus = $derived(smtpQuery.data?.smtpSettings ?? null);
	const exportRoot = $derived(boundaryQuery.data?.annotationSyncRoot ?? null);
	const emailers = $derived(emailersQuery.data?.emailers ?? []);
	const sender = $derived(emailers.find((emailer) => emailer.isPrimary) ?? emailers[0] ?? null);
	let name = $state('Coppice SMTP');
	let senderEmail = $state('');
	let senderDisplayName = $state('Coppice');
	let username = $state('');
	let password = $state('');
	let host = $state('');
	let port = $state('587');
	let tlsEnabled = $state(true);
	let recipient = $state('');
	let loadedSenderId = $state<number | null | undefined>(undefined);
	let saving = $state(false);
	let testing = $state(false);
	let testResult = $state<'success' | 'error' | null>(null);

	$effect(() => {
		if (loadedSenderId === sender?.id) return;
		loadedSenderId = sender?.id ?? null;
		if (!sender) return;
		name = sender.name;
		senderEmail = sender.senderEmail;
		senderDisplayName = sender.senderDisplayName;
		username = sender.username;
		host = sender.smtpHost;
		port = String(sender.smtpPort);
		tlsEnabled = sender.tlsEnabled;
	});

	function config(includePassword: boolean) {
		return {
			senderEmail: senderEmail.trim(),
			senderDisplayName: senderDisplayName.trim(),
			username: username.trim(),
			...(includePassword && password ? { password } : {}),
			host: host.trim(),
			port: Number(port),
			tlsEnabled
		};
	}

	function input() {
		return { name: name.trim(), isPrimary: true, config: config(true) };
	}

	function handleSaveSuccess(): void {
		password = '';
		loadedSenderId = undefined;
		void queryClient.invalidateQueries({ queryKey: ['admin-smtp'] });
		void queryClient.invalidateQueries({ queryKey: ['admin-smtp-status'] });
		toast.success('SMTP sender saved.');
	}

	function handleSaveError(error: unknown): void {
		toast.error(error instanceof Error ? error.message : 'Unable to save the SMTP sender.');
	}

	const createSender = createMutation(() => ({
		mutationFn: () => request(ConnectionCreateAdminEmailerDocument, { input: input() }),
		onSuccess: handleSaveSuccess,
		onError: handleSaveError,
		onSettled: () => (saving = false)
	}));

	const updateSender = createMutation(() => ({
		mutationFn: (id: number) => request(ConnectionUpdateAdminEmailerDocument, { id, input: input() }),
		onSuccess: handleSaveSuccess,
		onError: handleSaveError,
		onSettled: () => (saving = false)
	}));

	const test = createMutation(() => ({
		mutationFn: () => request(ConnectionTestAdminEmailerDocument, { config: config(true), recipient: recipient.trim() }),
		onSuccess: () => {
			testResult = 'success';
			password = '';
			toast.success('SMTP test accepted.');
		},
		onError: (error) => {
			testResult = 'error';
			toast.error(error instanceof Error ? error.message : 'SMTP test failed.');
		},
		onSettled: () => (testing = false)
	}));

	function saveSender(event: SubmitEvent): void {
		event.preventDefault();
		if (!senderEmail.trim() || !host.trim() || !Number.isInteger(Number(port))) return;
		saving = true;
		if (sender) updateSender.mutate(sender.id);
		else createSender.mutate();
	}

	function testSender(event: SubmitEvent): void {
		event.preventDefault();
		if (!recipient.trim() || !senderEmail.trim() || !host.trim()) return;
		testResult = null;
		testing = true;
		test.mutate();
	}
</script>

<div class="grid gap-5 xl:grid-cols-2">
	<Card>
		<CardHeader>
			<div class="flex flex-wrap items-start gap-2">
				<div class="mr-auto">
					<CardTitle class="text-base">SMTP sender</CardTitle>
					<CardDescription>One shared server sender powers Kindle delivery and server notifications.</CardDescription>
				</div>
				{#if emailersQuery.isPending || smtpQuery.isPending}
					<Badge variant="outline">Checking…</Badge>
				{:else if smtpStatus?.configured ?? Boolean(sender)}
					<Badge><CheckCircle2Icon aria-hidden="true" />Configured</Badge>
				{:else}
					<Badge variant="outline"><CircleAlertIcon aria-hidden="true" />Not configured</Badge>
				{/if}
			</div>
		</CardHeader>
		<CardContent>
			{#if smtpQuery.isError}
				<Alert variant="destructive" class="mb-4">
					<AlertTitle>Unable to load SMTP status</AlertTitle>
					<AlertDescription>
						{smtpQuery.error instanceof Error ? smtpQuery.error.message : 'Request failed.'}
					</AlertDescription>
					<Button type="button" size="sm" variant="outline" class="mt-3" onclick={() => smtpQuery.refetch()}>Retry</Button>
				</Alert>
			{/if}
			{#if emailersQuery.isPending}
				<div class="flex flex-col gap-3" aria-label="Loading SMTP configuration">
					<Skeleton class="h-10 w-full rounded-lg" />
					<Skeleton class="h-10 w-full rounded-lg" />
					<Skeleton class="h-10 w-full rounded-lg" />
				</div>
			{:else if emailersQuery.isError}
				<Alert variant="destructive">
					<AlertTitle>Unable to load SMTP configuration</AlertTitle>
					<AlertDescription>
						{emailersQuery.error instanceof Error ? emailersQuery.error.message : 'Request failed.'}
					</AlertDescription>
					<Button type="button" size="sm" variant="outline" class="mt-3" onclick={() => emailersQuery.refetch()}>Retry</Button>
				</Alert>
			{:else}
				<form class="grid gap-4" onsubmit={saveSender}>
					<div class="grid gap-3 sm:grid-cols-2">
						<div class="grid gap-2"><Label for="smtp-name">Configuration name</Label><Input id="smtp-name" bind:value={name} autocomplete="off" required /></div>
						<div class="grid gap-2"><Label for="smtp-sender-email">Sender email</Label><Input id="smtp-sender-email" bind:value={senderEmail} type="email" autocomplete="email" required /></div>
					</div>
					<div class="grid gap-3 sm:grid-cols-2">
						<div class="grid gap-2"><Label for="smtp-display-name">Sender display name</Label><Input id="smtp-display-name" bind:value={senderDisplayName} autocomplete="organization" required /></div>
						<div class="grid gap-2"><Label for="smtp-username">SMTP username</Label><Input id="smtp-username" bind:value={username} autocomplete="username" required /></div>
					</div>
					<div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_7rem]">
						<div class="grid gap-2"><Label for="smtp-host">SMTP host</Label><Input id="smtp-host" bind:value={host} autocomplete="url" required /></div>
						<div class="grid gap-2"><Label for="smtp-port">Port</Label><Input id="smtp-port" bind:value={port} inputmode="numeric" pattern="[0-9]+" required /></div>
					</div>
					<div class="grid gap-2"><Label for="smtp-password">SMTP password</Label><Input id="smtp-password" bind:value={password} type="password" autocomplete="new-password" placeholder={sender ? 'Stored — leave blank to keep' : 'Enter once'} /><span class="text-xs text-muted-foreground">Write-only. Coppice encrypts it before storage and never returns it.</span></div>
					<Label for="smtp-tls" class="flex items-center gap-2 text-sm font-normal"><Switch id="smtp-tls" bind:checked={tlsEnabled} />Use TLS</Label>
					<div class="flex flex-wrap items-center gap-2">
						<Button type="submit" disabled={saving || !senderEmail.trim() || !host.trim()}>{saving ? 'Saving…' : sender ? 'Save sender' : 'Configure sender'}</Button>
						{#if emailers.length > 1}<span class="text-xs text-muted-foreground">Only the primary sender is used by Connections.</span>{/if}
					</div>
				</form>
			{/if}
		</CardContent>
	</Card>

	<Card>
		<CardHeader>
			<CardTitle class="text-base">Test delivery</CardTitle>
			<CardDescription>Send a harmless test to verify host, credentials, TLS, and the approved sender.</CardDescription>
		</CardHeader>
		<CardContent>
			<form class="grid gap-4" onsubmit={testSender}>
				<div class="grid gap-2"><Label for="smtp-test-recipient">Test recipient</Label><Input id="smtp-test-recipient" bind:value={recipient} type="email" autocomplete="email" placeholder="you@example.com" required /></div>
				<p class="text-xs text-muted-foreground">The password field above is required for a first test when no stored password exists. It is cleared after the request.</p>
				<Button type="submit" variant="outline" disabled={testing || emailersQuery.isPending || !recipient.trim() || !sender}>{testing ? 'Sending…' : 'Send SMTP test'}</Button>
			</form>
			{#if testResult === 'success'}<Alert class="mt-4"><AlertTitle>Test accepted</AlertTitle><AlertDescription>Check the recipient inbox. Provider-side filtering can still delay or reject delivery.</AlertDescription></Alert>{/if}
			{#if testResult === 'error'}<Alert class="mt-4" variant="destructive"><AlertTitle>Test failed</AlertTitle><AlertDescription>Check the error toast, credentials, and TLS mode, then retry.</AlertDescription></Alert>{/if}
			{#if smtpStatus?.lastUsedAt ?? sender?.lastUsedAt}<p class="mt-4 text-xs text-muted-foreground">Last used {absoluteTime(smtpStatus?.lastUsedAt ?? sender?.lastUsedAt ?? '')}</p>{/if}
		</CardContent>
	</Card>

	<Card class="xl:col-span-2">
		<CardHeader>
			<div class="flex items-start gap-3">
				<ServerIcon class="mt-0.5 size-5 text-muted-foreground" aria-hidden="true" />
				<div>
					<CardTitle class="text-base">Mounted export root</CardTitle>
					<CardDescription>Annotation exports stay inside the administrator-mounted boundary.</CardDescription>
				</div>
			</div>
		</CardHeader>
		<CardContent>
			{#if boundaryQuery.isPending}
				<div class="flex flex-col gap-3" aria-label="Loading export boundary status">
					<Skeleton class="h-10 w-full rounded-lg" />
					<Skeleton class="h-10 w-full rounded-lg" />
				</div>
			{:else if boundaryQuery.isError}
				<Alert variant="destructive">
					<AlertTitle>Unable to load export boundary status</AlertTitle>
					<AlertDescription>
						{boundaryQuery.error instanceof Error ? boundaryQuery.error.message : 'Request failed.'}
					</AlertDescription>
					<Button type="button" size="sm" variant="outline" class="mt-3" onclick={() => boundaryQuery.refetch()}>Retry</Button>
				</Alert>
			{:else}
				<div class="grid gap-3 text-sm sm:grid-cols-2">
					<div class="rounded-lg border border-dashed bg-muted/30 p-3">
						<div class="flex flex-wrap items-center gap-2">
							<p class="font-medium text-foreground">Mounted path</p>
							<Badge variant={exportRoot ? 'default' : 'outline'}>{exportRoot ? 'Configured' : 'Not configured'}</Badge>
						</div>
						{#if exportRoot}
							<code class="mt-2 block break-all text-xs text-muted-foreground">{exportRoot}</code>
						{:else}
							<p class="mt-1 text-xs text-muted-foreground">Set STUMP_ANNOTATION_SYNC_ROOT and restart the server to enable exports.</p>
						{/if}
					</div>
					<div class="rounded-lg border border-dashed bg-muted/30 p-3">
						<p class="font-medium text-foreground">Shared metadata status</p>
						<p class="mt-1 text-xs text-muted-foreground">Provider metadata is shared as cacheable metadata only. Personal Hardcover tokens are never pooled across accounts.</p>
					</div>
				</div>
			{/if}
		</CardContent>
	</Card>
</div>

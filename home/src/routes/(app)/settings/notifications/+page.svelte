<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import {
		Card,
		CardContent,
		CardDescription,
		CardFooter,
		CardHeader,
		CardTitle
	} from '@stump/ui/components/ui/card';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { request } from '@stump/ui/graphql/client';
	import {
		NotificationSettingsDocument,
		SetNotificationChannelSettingsDocument,
		SetNotificationRuleDocument,
		TestNotificationChannelDocument,
		type IngestSettingValueType,
		type NotificationKind
	} from '$lib/graphql/generated/graphql';
	import { ALL_EVENT_KINDS, NOTIFICATION_KINDS } from '$lib/dashboard';
	import { getHomeSession } from '$lib/session.svelte';

	const session = getHomeSession();
	const queryClient = useQueryClient();

	const settingsQuery = createQuery(() => ({
		queryKey: ['notification-settings'],
		queryFn: () => request(NotificationSettingsDocument, {}),
		enabled: browser
	}));

	const channels = $derived(settingsQuery.data?.notificationChannels ?? []);
	const rules = $derived(settingsQuery.data?.notificationRules ?? []);
	// A `*` rule routes every kind to a channel. The server stores it, but
	// `setNotificationRule` only accepts one concrete kind, so it is reported
	// rather than edited here.
	const wildcards = $derived(
		rules.filter((rule) => rule.eventKind === ALL_EVENT_KINDS && rule.enabled)
	);
	const isOwner = $derived(Boolean(session.user?.isServerOwner));

	/** Draft values per channel, keyed by setting key. Secrets start empty. */
	let drafts = $state<Record<string, Record<string, string>>>({});
	let loadedFor = $state<string | null>(null);
	let savingChannel = $state<string | null>(null);
	let testingChannel = $state<string | null>(null);
	let pendingRule = $state<string | null>(null);

	// Seed the forms once per server response: the query returns the effective
	// values with secrets omitted, and re-seeding on every render would fight
	// the user's typing.
	$effect(() => {
		const data = settingsQuery.data;
		if (!data) return;
		const signature = data.notificationChannelSettings
			.map((entry) => entry.channelId)
			.sort()
			.join(',');
		if (signature === loadedFor) return;
		loadedFor = signature;
		drafts = Object.fromEntries(
			data.notificationChannels.map((channel) => {
				const stored = data.notificationChannelSettings.find(
					(entry) => entry.channelId === channel.id
				);
				const values = (stored?.values ?? {}) as Record<string, unknown>;
				return [
					channel.id,
					Object.fromEntries(
						channel.settings.map((definition) => [
							definition.key,
							definition.secret ? '' : stringify(values[definition.key])
						])
					)
				];
			})
		);
	});

	const saveSettings = createMutation(() => ({
		mutationFn: (channelId: string) =>
			request(SetNotificationChannelSettingsDocument, {
				input: { channelId, settings: serialize(channelId) }
			}),
		onSuccess: () => {
			loadedFor = null;
			void queryClient.invalidateQueries({ queryKey: ['notification-settings'] });
			toast.success('Channel settings saved.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to save channel settings.'),
		onSettled: () => (savingChannel = null)
	}));

	const testChannel = createMutation(() => ({
		mutationFn: (channelId: string) => request(TestNotificationChannelDocument, { channelId }),
		onSuccess: () => toast.success('Test notification delivered.'),
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'The channel rejected the test.'),
		onSettled: () => (testingChannel = null)
	}));

	const setRule = createMutation(() => ({
		mutationFn: (variables: { eventKind: NotificationKind; channelId: string; enabled: boolean }) =>
			request(SetNotificationRuleDocument, variables),
		onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['notification-settings'] }),
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to save the routing rule.'),
		onSettled: () => (pendingRule = null)
	}));

	function stringify(value: unknown): string {
		if (value === null || value === undefined) return '';
		if (typeof value === 'string') return value;
		if (typeof value === 'number' || typeof value === 'boolean') return String(value);
		return JSON.stringify(value);
	}

	function parse(value: string, type: IngestSettingValueType): unknown {
		if (type === 'BOOLEAN') return value === 'true';
		if (type === 'INTEGER') return Number.parseInt(value, 10);
		if (type === 'NUMBER') return Number(value);
		if (type === 'JSON') {
			try {
				return JSON.parse(value);
			} catch {
				return value;
			}
		}
		return value;
	}

	/**
	 * Only the channel's own keys, and only secrets the user actually typed:
	 * the server rejects unknown keys and keeps the stored (encrypted) value
	 * for any secret key that is omitted.
	 */
	function serialize(channelId: string): Record<string, unknown> {
		const channel = channels.find((candidate) => candidate.id === channelId);
		const draft = drafts[channelId] ?? {};
		return Object.fromEntries(
			(channel?.settings ?? [])
				.filter((definition) => !(definition.secret && draft[definition.key] === ''))
				.map((definition) => [
					definition.key,
					parse(draft[definition.key] ?? '', definition.valueType)
				])
		);
	}

	/**
	 * Drafts are written through this rather than `bind:value`: the seeding
	 * effect runs after the first render of the channel list, so an unseeded
	 * `drafts[channelId]` must never be indexed into.
	 */
	function setDraft(channelId: string, key: string, value: string): void {
		drafts[channelId] = { ...(drafts[channelId] ?? {}), [key]: value };
	}

	function ruleFor(kind: string, channelId: string): boolean {
		return rules.some(
			(rule) => rule.eventKind === kind && rule.channelId === channelId && rule.enabled
		);
	}

	function toggleRule(kind: NotificationKind, channelId: string, enabled: boolean): void {
		pendingRule = `${kind}:${channelId}`;
		setRule.mutate({ eventKind: kind, channelId, enabled });
	}

	function save(channelId: string): void {
		savingChannel = channelId;
		saveSettings.mutate(channelId);
	}

	function test(channelId: string): void {
		testingChannel = channelId;
		testChannel.mutate(channelId);
	}
</script>

<svelte:head>
	<title>Notifications · Stump</title>
</svelte:head>

<div class="flex flex-col gap-6">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">Notifications</h1>
		<p class="mt-1 max-w-2xl text-sm text-muted-foreground">
			Choose where this server tells you about your devices and its own work. Settings and rules
			are yours alone; secret values are write-only and never rendered back.
		</p>
	</div>

	{#if settingsQuery.isPending}
		<div class="flex flex-col gap-4">
			{#each { length: 3 } as _, index (index)}
				<Skeleton class="h-40 w-full rounded-xl" />
			{/each}
		</div>
	{:else if settingsQuery.isError}
		<Alert variant="destructive">
			<AlertTitle>Unable to load notification settings</AlertTitle>
			<AlertDescription>
				{settingsQuery.error instanceof Error
					? settingsQuery.error.message
					: 'The server did not return notification channels.'}
			</AlertDescription>
		</Alert>
	{:else if channels.length === 0}
		<Empty>
			<EmptyHeader>
				<EmptyTitle>No notification channels</EmptyTitle>
				<EmptyDescription>
					This server build ships no notification channels, so there is nothing to route.
				</EmptyDescription>
			</EmptyHeader>
		</Empty>
	{:else}
		<Card>
			<CardHeader>
				<CardTitle class="text-base">Routing rules</CardTitle>
				<CardDescription>
					One switch per event and channel.
					{#if !isOwner}
						Server-wide events (scans, ingest, metadata, analysis) are owner-only.
					{/if}
				</CardDescription>
			</CardHeader>
			<CardContent class="overflow-x-auto">
				<table class="w-full min-w-md text-sm">
					<thead class="text-left text-muted-foreground">
						<tr>
							<th class="py-1 font-medium">Event</th>
							{#each channels as channel (channel.id)}
								<th class="w-24 py-1 text-center font-medium">{channel.label}</th>
							{/each}
						</tr>
					</thead>
					<tbody>
						{#each NOTIFICATION_KINDS as event (event.kind)}
							{@const locked = event.administrative && !isOwner}
							<tr class="border-t align-top">
								<td class="py-2 pr-4">
									<span class="font-medium">{event.label}</span>
									{#if event.administrative}
										<Badge variant="outline" class="ml-2">server</Badge>
									{/if}
									<span class="block text-xs text-muted-foreground">{event.description}</span>
								</td>
								{#each channels as channel (channel.id)}
									<td class="py-2 text-center">
										<Switch
											aria-label={`${event.label} to ${channel.label}`}
											checked={ruleFor(event.kind, channel.id)}
											disabled={locked || pendingRule === `${event.kind}:${channel.id}`}
											onCheckedChange={(next) => toggleRule(event.kind, channel.id, next)}
										/>
									</td>
								{/each}
							</tr>
						{/each}
					</tbody>
				</table>
			</CardContent>
			{#if wildcards.length}
				<CardFooter>
					<p class="text-xs text-muted-foreground">
						A catch-all rule already routes every event to
						{wildcards
							.map(
								(rule) =>
									channels.find((channel) => channel.id === rule.channelId)?.label ??
									rule.channelId
							)
							.join(', ')}. The switches above add to it.
					</p>
				</CardFooter>
			{/if}
		</Card>

		{#each channels as channel (channel.id)}
			<Card>
				<CardHeader class="flex flex-wrap items-start gap-2">
					<div class="mr-auto">
						<CardTitle class="text-base">{channel.label}</CardTitle>
						<CardDescription>
							{channel.settings.length}
							{channel.settings.length === 1 ? 'setting' : 'settings'}
						</CardDescription>
					</div>
					<Badge variant="secondary">{channel.id}</Badge>
				</CardHeader>
				<CardContent>
					{#if channel.settings.length === 0}
						<p class="text-sm text-muted-foreground">This channel needs no configuration.</p>
					{:else}
						<div class="grid gap-5 md:grid-cols-2">
							{#each channel.settings as definition (definition.key)}
								{@const id = `${channel.id}-${definition.key}`}
								<div class="grid gap-2 text-sm">
									<label class="font-medium" for={id}>
										{definition.label}
										{#if definition.required}
											<span class="text-destructive" aria-hidden="true">*</span>
										{/if}
									</label>
									{#if definition.valueType === 'BOOLEAN'}
										<Switch
											{id}
											checked={drafts[channel.id]?.[definition.key] === 'true'}
											onCheckedChange={(next) =>
												setDraft(channel.id, definition.key, next ? 'true' : 'false')}
										/>
									{:else}
										<Input
											{id}
											type={definition.secret ? 'password' : 'text'}
											inputmode={definition.valueType === 'INTEGER' ||
											definition.valueType === 'NUMBER'
												? 'numeric'
												: undefined}
											autocomplete="off"
											placeholder={definition.secret
												? 'Stored — leave blank to keep'
												: stringify(definition.defaultValue)}
											value={drafts[channel.id]?.[definition.key] ?? ''}
											oninput={(event) =>
												setDraft(channel.id, definition.key, event.currentTarget.value)}
										/>
									{/if}
									{#if definition.description}
										<span class="text-xs text-muted-foreground">
											{definition.description}
											{#if definition.helpUrl}
												<a
													class="underline"
													href={definition.helpUrl}
													target="_blank"
													rel="noreferrer noopener">Docs</a
												>
											{/if}
										</span>
									{/if}
								</div>
							{/each}
						</div>
					{/if}
				</CardContent>
				<CardFooter class="flex flex-wrap gap-2">
					<Button
						size="sm"
						disabled={savingChannel === channel.id || channel.settings.length === 0}
						onclick={() => save(channel.id)}
					>
						{savingChannel === channel.id ? 'Saving…' : 'Save settings'}
					</Button>
					<Button
						size="sm"
						variant="outline"
						disabled={testingChannel === channel.id}
						onclick={() => test(channel.id)}
					>
						{testingChannel === channel.id ? 'Sending…' : 'Send test'}
					</Button>
				</CardFooter>
			</Card>
		{/each}
	{/if}
</div>

<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '$lib/components/ui/alert';
	import { Button } from '$lib/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
	import { Badge } from '$lib/components/ui/badge';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '$lib/components/ui/empty';
	import { Input } from '$lib/components/ui/input';
	import { Separator } from '$lib/components/ui/separator';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Switch } from '$lib/components/ui/switch';
	import * as Tabs from '$lib/components/ui/tabs';
	import * as Select from '$lib/components/ui/select';
	import { getEditorSession } from '$lib/editor/session.svelte';
	import { request } from '$lib/graphql/client';
	import {
		IngestProviderCatalogDocument,
		IngestProviderSettingsDocument,
		IngestQualityCheckCatalogDocument,
		SetIngestProviderSettingsDocument,
		SetIngestQualityCheckSettingsDocument,
		VerifyIngestProviderDocument,
		type IngestSettingValueType
	} from '$lib/graphql/generated/graphql';
	const queryClient = useQueryClient();
	let tab = $state('providers');
	let selectedProviderId = $state('');
	type ProviderSetting = { secret: boolean; value: unknown };
	let providerEnabled = $state(false);
	let providerOptedIn = $state(false);
	let providerValues = $state<Record<string, string>>({});
	let providerStateLoadedFor = $state<string | null>(null);

	const providerCatalogQuery = createQuery(() => ({
		queryKey: ['provider-catalog'],
		queryFn: () => request(IngestProviderCatalogDocument, { includeDisabled: true }),
		enabled: browser
	}));
	const qualityCatalogQuery = createQuery(() => ({
		queryKey: ['quality-catalog'],
		queryFn: () => request(IngestQualityCheckCatalogDocument, { includeDisabled: true }),
		enabled: browser
	}));
	const providerSettingsQuery = createQuery(() => ({
		queryKey: ['provider-settings', selectedProviderId],
		queryFn: () => request(IngestProviderSettingsDocument, { providerId: selectedProviderId }),
		enabled: browser && Boolean(selectedProviderId)
	}));
	const saveProviderMutation = createMutation(() => ({
		mutationFn: () =>
			request(SetIngestProviderSettingsDocument, {
				input: {
					providerId: selectedProviderId,
					enabled: providerEnabled,
					optedIn: providerOptedIn,
					settings: serializeSettings()
				}
			}),
		onSuccess: () => {
				void queryClient.invalidateQueries({ queryKey: ['provider-settings', selectedProviderId] });
				void queryClient.invalidateQueries({ queryKey: ['provider-catalog'] });
				toast.success('Provider settings saved.');
			},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to save provider settings.')
	}));
	const verifyMutation = createMutation(() => ({
		mutationFn: () => request(VerifyIngestProviderDocument, { providerId: selectedProviderId, settings: serializeSettings() }),
		onSuccess: (result) => {
			if (result.verifyIngestProvider.isValid) toast.success('Provider credentials verified.');
			else toast.error(result.verifyIngestProvider.error ?? 'Provider credentials were rejected.');
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Provider verification failed.')
	}));
	const saveQualityMutation = createMutation(() => ({
		mutationFn: ({ checkId, enabled }: { checkId: string; enabled: boolean }) =>
			request(SetIngestQualityCheckSettingsDocument, { input: { checkId, enabled, settings: {} } }),
		onSuccess: () => {
				void queryClient.invalidateQueries({ queryKey: ['quality-catalog'] });
				toast.success('Quality-check setting saved.');
			},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to save quality-check setting.')
	}));

	let providers = $derived(providerCatalogQuery.data?.ingestProviderCatalog ?? []);
	let checks = $derived(qualityCatalogQuery.data?.ingestQualityCheckCatalog ?? []);
	let selectedProvider = $derived(providers.find((provider) => provider.id === selectedProviderId));
	let providerSettings = $derived(providerSettingsQuery.data?.ingestProviderSettings);

	$effect(() => {
		if (!selectedProviderId && providers.length) selectedProviderId = providers[0].id;
	});
	$effect(() => {
		const state = providerSettings;
		if (!state || state.provider.id === providerStateLoadedFor) return;
		providerStateLoadedFor = state.provider.id;
		providerEnabled = state.enabled;
		providerOptedIn = state.optedIn;
		providerValues = Object.fromEntries(
			state.settings.filter((setting) => !setting.secret && setting.value !== null).map((setting) => [setting.key, stringifyValue(setting.value)])
		);
	});

	function stringifyValue(value: unknown): string {
		if (typeof value === 'string') return value;
		return JSON.stringify(value ?? '');
	}

	function parseValue(value: string, type: IngestSettingValueType): unknown {
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

	function serializeSettings(): Record<string, unknown> {
		const definitions = selectedProvider?.settings ?? [];
		return Object.fromEntries(
			definitions
				.filter((definition) => providerValues[definition.key]?.trim())
				.map((definition) => [definition.key, parseValue(providerValues[definition.key], definition.valueType)])
		);
	}

	function selectProvider(providerId: string | undefined): void {
		if (!providerId) return;
		selectedProviderId = providerId;
		providerStateLoadedFor = null;
		providerValues = {};
	}

	function settingValue(setting: ProviderSetting | undefined): string {
		return setting?.secret ? '' : stringifyValue(setting?.value);
	}
</script>

<svelte:head><title>Settings · Stump ingest</title></svelte:head>

<div class="flex flex-col gap-6">
	<div>
		<p class="text-sm font-medium text-primary">Registry and policy</p>
		<h1 class="text-3xl font-semibold tracking-tight">Ingest settings</h1>
		<p class="mt-1 max-w-3xl text-muted-foreground">Configure metadata providers and deterministic quality checks. Secret values are write-only and never rendered back from the server.</p>
	</div>
	<Tabs.Root bind:value={tab}>
		<Tabs.List class="h-auto w-fit"><Tabs.Trigger value="providers">Metadata providers</Tabs.Trigger><Tabs.Trigger value="quality">Quality checks</Tabs.Trigger></Tabs.List>
		<Tabs.Content value="providers" class="flex flex-col gap-6">
			<Card>
				<CardHeader>
					<CardTitle>Provider catalog</CardTitle>
					<CardDescription>Descriptors come from the server registry, including providers that are not configured yet.</CardDescription>
				</CardHeader>
				<CardContent>
					{#if providerCatalogQuery.isPending}
						<Skeleton class="h-10 w-full" />
					{:else if providerCatalogQuery.isError}
						<Alert variant="destructive">
							<AlertTitle>Unable to load providers</AlertTitle>
							<AlertDescription>{providerCatalogQuery.error instanceof Error ? providerCatalogQuery.error.message : 'The server did not return provider descriptors.'}</AlertDescription>
						</Alert>
					{:else if !providers.length}
						<Empty>
							<EmptyHeader>
								<EmptyTitle>No providers registered</EmptyTitle>
								<EmptyDescription>Install or enable a provider in the server build before configuring it.</EmptyDescription>
							</EmptyHeader>
						</Empty>
					{:else}
						<div class="flex flex-col gap-4">
							<div class="grid max-w-xl gap-2 text-sm font-medium">
								<span>Provider</span>
								<Select.Root
									type="single"
									name="provider"
									bind:value={selectedProviderId}
									onValueChange={selectProvider}
								>
									<Select.Trigger class="w-full">
										{selectedProvider?.name ?? 'Select a provider'}
									</Select.Trigger>
									<Select.Content>
										<Select.Group>
											<Select.Label>Metadata providers</Select.Label>
											{#each providers as provider (provider.id)}
												<Select.Item
													value={provider.id}
													label={`${provider.name} · ${provider.id}`}
												>
													{provider.name} · {provider.id}
												</Select.Item>
											{/each}
										</Select.Group>
									</Select.Content>
								</Select.Root>
							</div>
							<div class="grid gap-3 md:grid-cols-3" aria-label="Provider catalog">
								{#each providers as provider (provider.id)}
									<Button
										type="button"
										variant={provider.id === selectedProviderId ? 'secondary' : 'outline'}
										class="h-auto min-h-16 w-full justify-start text-left"
										onclick={() => selectProvider(provider.id)}
									>
										<span class="flex min-w-0 flex-1 flex-col items-start gap-1">
											<span class="truncate font-medium">{provider.name}</span>
											<span class="truncate text-xs text-muted-foreground">{provider.id}</span>
										</span>
										<Badge variant={provider.configured ? 'secondary' : 'outline'}>
											{provider.configured ? 'Configured' : 'Not configured'}
										</Badge>
									</Button>
								{/each}
							</div>
						</div>
					{/if}
				</CardContent>
			</Card>
			{#if selectedProvider}
				<Card>
					<CardHeader><div class="flex flex-wrap items-start justify-between gap-4"><div><CardTitle>{selectedProvider.name}</CardTitle><CardDescription>Version {selectedProvider.version} · {selectedProvider.capabilities.join(', ') || 'No capabilities'}</CardDescription></div><div class="flex items-center gap-4"><label class="flex items-center gap-2 text-sm">Enabled<Switch checked={providerEnabled} onchange={(event) => (providerEnabled = Boolean((event.currentTarget as HTMLButtonElement).getAttribute('data-state') === 'checked'))} /></label><label class="flex items-center gap-2 text-sm">Opted in<Switch checked={providerOptedIn} onchange={(event) => (providerOptedIn = Boolean((event.currentTarget as HTMLButtonElement).getAttribute('data-state') === 'checked'))} /></label></div></div></CardHeader>
					<CardContent>
						{#if providerSettingsQuery.isPending}<div class="flex flex-col gap-3"><Skeleton class="h-10 w-full" /><Skeleton class="h-10 w-full" /></div>{:else if providerSettingsQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load provider settings</AlertTitle><AlertDescription>{providerSettingsQuery.error instanceof Error ? providerSettingsQuery.error.message : 'The server did not return provider settings.'}</AlertDescription></Alert>{:else}<div class="grid gap-5 md:grid-cols-2">{#each selectedProvider.settings as definition (definition.key)}{@const setting = providerSettings?.settings.find((current) => current.key === definition.key)}<label class="grid gap-2 text-sm font-medium" for={`provider-${definition.key}`}>{definition.label}{#if definition.secret}<span class="text-xs font-normal text-muted-foreground">{setting?.configured ? 'Configured; leave blank to keep the current secret.' : 'Write-only secret.'}</span>{/if}<Input id={`provider-${definition.key}`} type={definition.secret ? 'password' : definition.valueType === 'JSON' ? 'text' : definition.valueType === 'BOOLEAN' ? 'text' : definition.valueType === 'INTEGER' || definition.valueType === 'NUMBER' ? 'number' : 'text'} value={providerValues[definition.key] ?? settingValue(setting)} oninput={(event) => (providerValues = { ...providerValues, [definition.key]: (event.currentTarget as HTMLInputElement).value })} required={definition.required && !setting?.configured} autocomplete={definition.secret ? 'new-password' : undefined} /><span class="text-xs font-normal text-muted-foreground">{definition.description ?? `Expected ${definition.valueType.toLowerCase()} value.`}</span></label>{/each}</div><div class="mt-6 flex flex-wrap justify-end gap-2"><Button variant="outline" disabled={!selectedProvider.configured || verifyMutation.isPending} onclick={() => verifyMutation.mutate()}>{verifyMutation.isPending ? 'Verifying…' : 'Verify provider'}</Button><Button disabled={saveProviderMutation.isPending} onclick={() => saveProviderMutation.mutate()}>{saveProviderMutation.isPending ? 'Saving…' : 'Save provider settings'}</Button></div>{/if}
					</CardContent>
				</Card>
			{/if}
		</Tabs.Content>
		<Tabs.Content value="quality" class="flex flex-col gap-4">
			{#if qualityCatalogQuery.isPending}{#each Array(5) as _, index (index)}<Skeleton class="h-24 w-full" />{/each}{:else if qualityCatalogQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load quality checks</AlertTitle><AlertDescription>{qualityCatalogQuery.error instanceof Error ? qualityCatalogQuery.error.message : 'The server did not return quality descriptors.'}</AlertDescription></Alert>{:else if !checks.length}<Empty><EmptyHeader><EmptyTitle>No quality checks registered</EmptyTitle><EmptyDescription>The server build has no ingest quality checks available.</EmptyDescription></EmptyHeader></Empty>{:else}{#each checks as check (check.id)}<Card><CardContent class="flex flex-wrap items-start justify-between gap-4 p-5"><div><div class="flex items-center gap-2"><h2 class="font-medium">{check.name}</h2><span class="rounded bg-muted px-2 py-0.5 text-xs tabular-nums">weight {check.weight}</span></div><p class="mt-1 text-sm text-muted-foreground">{check.id} · version {check.version} · {check.supportedMediaTypes.join(', ') || 'all formats'}</p>{#if check.settings.length}<p class="mt-2 text-xs text-muted-foreground">Settings: {check.settings.map((setting) => setting.label).join(', ')}</p>{/if}</div><label class="flex items-center gap-2 text-sm">Enabled<Switch checked={check.enabled} onchange={(event) => saveQualityMutation.mutate({ checkId: check.id, enabled: (event.currentTarget as HTMLButtonElement).getAttribute('data-state') === 'checked' })} /></label></CardContent></Card>{/each}{/if}
		</Tabs.Content>
	</Tabs.Root>
	<Separator />
	<p class="text-sm text-muted-foreground">Changing a check setting invalidates affected reports and requeues items; the server preserves prior algorithm-versioned evidence.</p>
</div>

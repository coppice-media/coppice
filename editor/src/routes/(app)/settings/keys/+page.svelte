<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import type { ResultOf } from '@graphql-typed-document-node/core';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '$lib/components/ui/card';
	import { Input } from '$lib/components/ui/input';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Switch } from '$lib/components/ui/switch';
	import { request } from '$lib/graphql/client';
	import {
		CreateMetadataProviderDocument,
		DeleteMetadataProviderDocument,
		IngestProviderCatalogDocument,
		IngestProviderSettingsDocument,
		MetadataProviderConfigsDocument,
		SetIngestProviderSettingsDocument,
		UpdateMetadataProviderDocument,
		VerifyIngestProviderDocument
	} from '$lib/graphql/generated/graphql';

	const queryClient = useQueryClient();

	type CatalogEntry = ResultOf<typeof IngestProviderCatalogDocument>['ingestProviderCatalog'][number];
	type ProviderSettings = ResultOf<typeof IngestProviderSettingsDocument>['ingestProviderSettings'];
	type ProviderConfig = ResultOf<typeof MetadataProviderConfigsDocument>['metadataProviderConfigs'][number];

	type VerifyResult = { ok: boolean; message: string };

	let nativeDrafts = $state<Record<string, string>>({});
	let replacing = $state<Record<string, boolean>>({});
	let verifyResults = $state<Record<string, VerifyResult>>({});
	let noKeyExpanded = $state(false);

	const keysQuery = createQuery(() => ({
		queryKey: ['api-keys'],
		queryFn: async () => {
			const catalog = await request(IngestProviderCatalogDocument, { includeDisabled: true });
			const configs = await request(MetadataProviderConfigsDocument, {});
			const nativeIds = catalog.ingestProviderCatalog
				.filter((provider) => provider.settings.some((setting) => setting.secret))
				.map((provider) => provider.id);
			const settingsEntries = await Promise.all(
				nativeIds.map(async (providerId) => {
					const settings = await request(IngestProviderSettingsDocument, { providerId });
					return [providerId, settings.ingestProviderSettings] as const;
				})
			);
			return {
				providers: catalog.ingestProviderCatalog,
				configs: configs.metadataProviderConfigs,
				nativeSettings: Object.fromEntries(settingsEntries) as Record<string, ProviderSettings>
			};
		},
		enabled: browser
	}));

	function invalidateKeys(): void {
		void queryClient.invalidateQueries({ queryKey: ['api-keys'] });
	}

	function invalidateCatalog(): void {
		void queryClient.invalidateQueries({ queryKey: ['provider-catalog'] });
	}

	const saveNativeMutation = createMutation(() => ({
		mutationFn: (input: { providerId: string; enabled?: boolean; settings?: Record<string, unknown> }) =>
			request(SetIngestProviderSettingsDocument, {
				input: {
					providerId: input.providerId,
					enabled: input.enabled,
					optedIn: undefined,
					settings: input.settings
				}
			}),
		onSuccess: (_data, variables) => {
			if (variables.settings) {
				for (const settingKey of Object.keys(variables.settings)) {
					delete nativeDrafts[draftKey(variables.providerId, settingKey)];
				}
			}
			invalidateKeys();
			invalidateCatalog();
		}
	}));

	const verifyMutation = createMutation(() => ({
		mutationFn: (input: { providerId: string; settings?: Record<string, unknown> }) =>
			request(VerifyIngestProviderDocument, { providerId: input.providerId, settings: input.settings }),
		onSuccess: (result, variables) => {
			verifyResults[variables.providerId] = result.verifyIngestProvider.isValid
				? { ok: true, message: 'Credentials verified.' }
				: {
						ok: false,
						message: result.verifyIngestProvider.error ?? 'Credentials were rejected.'
					};
		},
		onError: (error, variables) => {
			verifyResults[variables.providerId] = {
				ok: false,
				message: error instanceof Error ? error.message : 'Credential verification failed.'
			};
		}
	}));

	const saveConfigTokenMutation = createMutation(() => ({
		mutationFn: async (input: {
			provider: CatalogEntry;
			apiToken: string;
			configId?: number;
		}): Promise<number> => {
			if (input.configId === undefined) {
				const result = await request(CreateMetadataProviderDocument, {
					input: {
						providerType: input.provider.id.toUpperCase() as ProviderConfig['providerType'],
						apiToken: input.apiToken,
						enabled: true
					}
				});
				return result.createMetadataProvider.id;
			}
			const result = await request(UpdateMetadataProviderDocument, {
				id: input.configId,
				input: { apiToken: input.apiToken }
			});
			return result.updateMetadataProvider.id;
		},
		onSuccess: (_data, variables) => {
			for (const definition of secretSettings(variables.provider)) {
				delete nativeDrafts[draftKey(variables.provider.id, definition.key)];
			}
			invalidateKeys();
			invalidateCatalog();
		}
	}));

	const clearConfigMutation = createMutation(() => ({
		mutationFn: (configId: number) => request(DeleteMetadataProviderDocument, { id: configId }),
		onSuccess: () => {
			invalidateKeys();
			invalidateCatalog();
		}
	}));

	const toggleConfigEnabledMutation = createMutation(() => ({
		mutationFn: (input: { configId: number; enabled: boolean }) =>
			request(UpdateMetadataProviderDocument, {
				id: input.configId,
				input: { enabled: input.enabled }
			}),
		onSuccess: invalidateKeys
	}));

	let data = $derived(keysQuery.data);
	let keyProviders = $derived(
		(data?.providers ?? []).filter(
			(provider) =>
				provider.settings.some((setting) => setting.secret) ||
				configRowFor(provider) !== undefined ||
				provider.requiresApiToken
		)
	);
	let noKeyProviders = $derived(
		(data?.providers ?? []).filter(
			(provider) => !keyProviders.some((keyed) => keyed.id === provider.id)
		)
	);

	function secretSettings(provider: CatalogEntry) {
		return provider.settings.filter((setting) => setting.secret);
	}

	function configRowFor(provider: CatalogEntry): ProviderConfig | undefined {
		return data?.configs.find((row) => row.providerType.toLowerCase() === provider.id);
	}

	function nativeSettingsFor(provider: CatalogEntry): ProviderSettings | undefined {
		return data?.nativeSettings[provider.id];
	}

	function enabledFor(provider: CatalogEntry): boolean {
		const row = configRowFor(provider);
		if (row) return row.enabled;
		return nativeSettingsFor(provider)?.enabled ?? provider.enabledByDefault;
	}

	function draftKey(providerId: string, settingKey: string): string {
		return `${providerId}:${settingKey}`;
	}

	function draftFor(provider: CatalogEntry, settingKey: string): string {
		return nativeDrafts[draftKey(provider.id, settingKey)] ?? '';
	}

	function isConfigured(provider: CatalogEntry, settingKey: string): boolean {
		const row = configRowFor(provider);
		if (row) return true;
		const setting = nativeSettingsFor(provider)?.settings.find((current) => current.key === settingKey);
		return setting?.configured ?? false;
	}

	function changedNativeSettings(provider: CatalogEntry): Record<string, string> | undefined {
		const changed = Object.fromEntries(
			secretSettings(provider)
				.map((definition) => [definition.key, draftFor(provider, definition.key).trim()])
				.filter(([, value]) => Boolean(value))
		);
		return Object.keys(changed).length ? changed : undefined;
	}

	function hasNativeChanges(provider: CatalogEntry): boolean {
		return changedNativeSettings(provider) !== undefined;
	}

	function saveNative(provider: CatalogEntry): void {
		saveNativeMutation.mutate({ providerId: provider.id, settings: changedNativeSettings(provider) });
	}

	function clearNative(provider: CatalogEntry, settingKey: string): void {
		saveNativeMutation.mutate({
			providerId: provider.id,
			settings: { [settingKey]: '' }
		});
	}

	function saveConfigToken(provider: CatalogEntry): void {
		const row = configRowFor(provider);
		const apiToken = secretSettings(provider)
			.map((definition) => draftFor(provider, definition.key).trim())
			.find(Boolean);
		if (!apiToken) return;
		saveConfigTokenMutation.mutate({
			provider,
			apiToken,
			configId: row?.id
		});
	}

	function hasConfigDraft(provider: CatalogEntry): boolean {
		return secretSettings(provider).some((definition) => Boolean(draftFor(provider, definition.key).trim()));
	}

	function verifyProvider(provider: CatalogEntry): void {
		verifyMutation.mutate({
			providerId: provider.id,
			settings: configRowFor(provider) ? undefined : changedNativeSettings(provider)
		});
	}
</script>

<svelte:head><title>Settings · API keys</title></svelte:head>

<div class="flex flex-col gap-6">
	<div>
		<p class="text-sm font-medium text-primary">Credentials</p>
		<h1 class="text-3xl font-semibold tracking-tight">API keys</h1>
		<p class="mt-1 max-w-3xl text-muted-foreground">
			Enter provider credentials once. Secret values are stored encrypted on the server and are never rendered back.
		</p>
	</div>
	{#if keysQuery.isPending}
		{#each Array(3) as _, index (index)}
			<Skeleton class="h-32 w-full" />
		{/each}
	{:else if keysQuery.isError}
		<p class="text-sm text-destructive">
			{keysQuery.error instanceof Error ? keysQuery.error.message : 'Unable to load API keys.'}
		</p>
	{:else if data}
		{#each keyProviders as provider (provider.id)}
			{@const row = configRowFor(provider)}
			{@const usesConfigStore = row !== undefined || (!secretSettings(provider).length && provider.requiresApiToken)}
			{@const definitions = usesConfigStore
				? [{ key: provider.id, label: 'API key', helpUrl: provider.helpUrl }]
				: secretSettings(provider).map((definition) => ({ key: definition.key, label: definition.label, helpUrl: definition.helpUrl }))}
			<Card>
				<CardHeader>
					<div class="flex flex-wrap items-start justify-between gap-4">
						<div>
							<CardTitle class="flex items-center gap-2">
								{provider.name}
								<Badge variant="secondary">
									{definitions.every((definition) => isConfigured(provider, definition.key)) ? '•••• configured' : 'Not configured'}
								</Badge>
							</CardTitle>
							<CardDescription>
								{usesConfigStore
									? 'Credential is stored encrypted in the server configuration.'
									: `Credential${definitions.length > 1 ? 's' : ''}: ${definitions.map((definition) => definition.label).join(', ')}.`}
							</CardDescription>
						</div>
						<label class="flex items-center gap-2 text-sm">
							Enabled
							<Switch
								checked={enabledFor(provider)}
								disabled={usesConfigStore && row === undefined}
								onchange={(event) => {
									const enabled = (event.currentTarget as HTMLButtonElement).getAttribute('data-state') === 'checked';
									if (row) toggleConfigEnabledMutation.mutate({ configId: row.id, enabled });
									else saveNativeMutation.mutate({ providerId: provider.id, enabled });
								}}
							/>
						</label>
					</div>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					{#each definitions as definition (definition.key)}
						<div class="grid gap-2">
							<div class="flex flex-wrap items-center gap-3">
								<label class="text-sm font-medium" for={`key-${provider.id}-${definition.key}`}>
									{definition.label}
								</label>
								{#if definition.helpUrl}
									<a
										class="text-sm text-primary underline-offset-4 hover:underline"
										href={definition.helpUrl}
										target="_blank"
										rel="noreferrer"
									>
										Get a key →
									</a>
								{/if}
							</div>
							<div class="flex flex-wrap items-center gap-2">
								<Input
									id={`key-${provider.id}-${definition.key}`}
									class="max-w-sm"
									type="password"
									autocomplete="new-password"
									disabled={isConfigured(provider, definition.key) && !replacing[draftKey(provider.id, definition.key)]}
									placeholder={isConfigured(provider, definition.key) ? '••••••••' : 'Enter key'}
									value={draftFor(provider, definition.key)}
									oninput={(event) =>
										(nativeDrafts[draftKey(provider.id, definition.key)] =
											(event.currentTarget as HTMLInputElement).value)}
								/>
								{#if isConfigured(provider, definition.key)}
									<Button
										variant="outline"
										size="sm"
										onclick={() => (replacing[draftKey(provider.id, definition.key)] = true)}
									>
										Replace
									</Button>
									<Button
										variant="ghost"
										size="sm"
										disabled={saveNativeMutation.isPending || clearConfigMutation.isPending}
										onclick={() =>
											row && usesConfigStore
												? clearConfigMutation.mutate(row.id)
												: clearNative(provider, definition.key)}
									>
										Clear
									</Button>
								{/if}
							</div>
						</div>
					{/each}
					<div class="flex flex-wrap items-center gap-2">
						<Button
							variant="outline"
							disabled={verifyMutation.isPending || (usesConfigStore && row === undefined)}
							onclick={() => verifyProvider(provider)}
						>
							{verifyMutation.isPending ? 'Verifying…' : 'Verify'}
						</Button>
						{#if usesConfigStore}
							<Button
								disabled={!hasConfigDraft(provider) || saveConfigTokenMutation.isPending}
								onclick={() => saveConfigToken(provider)}
							>
								{saveConfigTokenMutation.isPending ? 'Saving…' : 'Save key'}
							</Button>
						{:else}
							<Button
								disabled={!hasNativeChanges(provider) || saveNativeMutation.isPending}
								onclick={() => saveNative(provider)}
							>
								{saveNativeMutation.isPending ? 'Saving…' : 'Save key'}
							</Button>
						{/if}
						{#if verifyResults[provider.id]}
							<span class="text-sm {verifyResults[provider.id].ok ? 'text-emerald-600' : 'text-destructive'}">
								{verifyResults[provider.id].message}
							</span>
						{/if}
					</div>
				</CardContent>
			</Card>
		{/each}
		{#if noKeyProviders.length}
			<Card>
				<CardHeader>
					<button
						class="flex w-full items-center justify-between text-left"
						type="button"
						onclick={() => (noKeyExpanded = !noKeyExpanded)}
						aria-expanded={noKeyExpanded}
					>
						<div>
							<CardTitle>No key needed</CardTitle>
							<CardDescription>
								{noKeyProviders.length} provider{noKeyProviders.length > 1 ? 's' : ''} that run without credentials.
							</CardDescription>
						</div>
						<span class="text-sm text-muted-foreground">{noKeyExpanded ? 'Hide' : 'Show'}</span>
					</button>
				</CardHeader>
				{#if noKeyExpanded}
					<CardContent class="flex flex-col gap-3">
						{#each noKeyProviders as provider (provider.id)}
							<div class="flex flex-wrap items-center justify-between gap-4">
								<div>
									<p class="text-sm font-medium">{provider.name}</p>
									<p class="text-xs text-muted-foreground">{provider.supportedMediaTypes.join(', ') || 'All media'}</p>
								</div>
								<label class="flex items-center gap-2 text-sm">
									Enabled
									<Switch
										checked={enabledFor(provider)}
										onchange={(event) =>
											saveNativeMutation.mutate({
												providerId: provider.id,
												enabled:
													(event.currentTarget as HTMLButtonElement).getAttribute('data-state') !== 'checked'
											})}
									/>
								</label>
							</div>
						{/each}
					</CardContent>
				{/if}
			</Card>
		{/if}
	{/if}
</div>

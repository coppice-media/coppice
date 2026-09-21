<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import { Separator } from '@stump/ui/components/ui/separator';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Switch } from '@stump/ui/components/ui/switch';
	import * as Tabs from '@stump/ui/components/ui/tabs';
	import * as Select from '@stump/ui/components/ui/select';
	import { getEditorSession } from '$lib/editor/session.svelte';
	import { request } from '@stump/ui/graphql/client';
	import {
		IngestProviderCatalogDocument,
		IngestProviderSettingsDocument,
		IngestQualityCheckCatalogDocument,
		MetadataPolicyDocument,
		SetIngestProviderSettingsDocument,
		SetIngestQualityCheckSettingsDocument,
		SetMetadataPolicyDocument,
		VerifyIngestProviderDocument,
		type IngestSettingValueType,
		type MetadataField,
		type MetadataPolicyFieldsFragment,
		type MetadataPolicyStrategy
	} from '$lib/graphql/generated/graphql';
	const queryClient = useQueryClient();
	let tab = $state('providers');
	let selectedProviderId = $state('');
	type ProviderSetting = { secret: boolean; value: unknown };
	let providerEnabled = $state(false);
	let providerOptedIn = $state(false);
	let providerValues = $state<Record<string, string>>({});
	let providerStateLoadedFor = $state<string | null>(null);

	type PolicyRow = MetadataPolicyFieldsFragment['fields'][number];
	const STRATEGIES: { value: MetadataPolicyStrategy; label: string; hint: string }[] = [
		{ value: 'FIRST', label: 'First match', hint: 'The first provider in the list that has a value wins.' },
		{ value: 'MERGE_UNION', label: 'Merge (union)', hint: 'Union of the stored value and every provider, deduped case-insensitively. List fields only.' },
		{ value: 'LONGEST', label: 'Longest', hint: 'The longest value wins; ties keep the higher-priority provider.' },
		{ value: 'PREFER_EXISTING', label: 'Prefer existing', hint: 'Only fill the field when it is currently empty.' },
		{ value: 'HIGHEST_RESOLUTION', label: 'Highest resolution', hint: 'The largest advertised cover wins. Cover only.' }
	];
	let selectedLibraryId = $state('');
	let policyRows = $state<PolicyRow[]>([]);
	let policyDirty = $state<string[]>([]);
	let policyLoadedFor = $state<string | null>(null);
	// The audio section is stored and overridden as a whole, so one dirty
	// flag covers it rather than one per key.
	type AudioPolicy = MetadataPolicyFieldsFragment['audio'];
	let audioPolicy = $state<AudioPolicy | null>(null);
	let audioDirty = $state(false);
	const session = getEditorSession();

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
	const policyQuery = createQuery(() => ({
		queryKey: ['metadata-policy', selectedLibraryId],
		queryFn: () => request(MetadataPolicyDocument, { libraryId: selectedLibraryId }),
		enabled: browser && Boolean(selectedLibraryId)
	}));
	// `null` audio clears the section back to the server default, exactly as
	// an empty field list clears the whole override.
	const savePolicyMutation = createMutation(() => ({
		mutationFn: ({ fields, audio }: { fields: PolicyRow[]; audio: AudioPolicy | null }) =>
			request(SetMetadataPolicyDocument, {
				libraryId: selectedLibraryId,
				input: {
					fields: fields.map((row) => ({
						field: row.field,
						providers: row.providers,
						strategy: row.strategy,
						lockRespected: row.lockRespected
					})),
					audio: audio
						? {
								singleFileWeight: audio.singleFileWeight,
								autoAssemble: audio.autoAssemble,
								autoChapters: audio.autoChapters,
								keepOriginal: audio.keepOriginal
							}
						: null
				}
			}),
		onSuccess: (result) => {
			policyLoadedFor = null;
			policyDirty = [];
			audioDirty = false;
			// The mutation answers with the same shape the query reads, but
			// under its own field: unwrap it or the cache write silently
			// blanks the tab.
			queryClient.setQueryData(['metadata-policy', selectedLibraryId], {
				metadataPolicy: result.setMetadataPolicy
			});
			toast.success(
				result.setMetadataPolicy.hasLibraryOverride
					? 'Library policy saved.'
					: 'Library policy cleared; the server default applies.'
			);
		},
		onError: (error) => toast.error(error instanceof Error ? error.message : 'Unable to save the metadata policy.')
	}));

	let providers = $derived(providerCatalogQuery.data?.ingestProviderCatalog ?? []);
	let checks = $derived(qualityCatalogQuery.data?.ingestQualityCheckCatalog ?? []);
	let selectedProvider = $derived(providers.find((provider) => provider.id === selectedProviderId));
	let providerSettings = $derived(providerSettingsQuery.data?.ingestProviderSettings);
	let libraries = $derived(session.libraries);
	let policy = $derived(policyQuery.data?.metadataPolicy);
	let policyProviderIds = $derived(providers.map((provider) => provider.id));
	let policyChanged = $derived(policyDirty.length > 0 || audioDirty);

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
	$effect(() => {
		if (!selectedLibraryId && libraries.length) {
			selectedLibraryId = session.selectedLibraryId || libraries[0].id;
		}
	});
	$effect(() => {
		const loaded = policy;
		if (!loaded || loaded.libraryId === policyLoadedFor) return;
		policyLoadedFor = loaded.libraryId;
		policyDirty = [];
		audioDirty = false;
		policyRows = loaded.fields.map((row) => ({ ...row, providers: [...row.providers] }));
		audioPolicy = { ...loaded.audio };
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

	function selectLibrary(libraryId: string | undefined): void {
		if (!libraryId) return;
		selectedLibraryId = libraryId;
		policyLoadedFor = null;
		policyDirty = [];
	}

	function fieldLabel(field: MetadataField): string {
		return field
			.toLowerCase()
			.split('_')
			.map((part) => part.charAt(0).toUpperCase() + part.slice(1))
			.join(' ');
	}

	function strategyLabel(strategy: MetadataPolicyStrategy): string {
		return STRATEGIES.find((entry) => entry.value === strategy)?.label ?? strategy;
	}

	// Mark a field as locally edited and hand back a fresh row array so the
	// rune tracks the change.
	function updateRow(field: MetadataField, change: (row: PolicyRow) => PolicyRow): void {
		policyRows = policyRows.map((row) => (row.field === field ? change(row) : row));
		if (!policyDirty.includes(field)) policyDirty = [...policyDirty, field];
	}

	function moveProvider(field: MetadataField, index: number, direction: -1 | 1): void {
		updateRow(field, (row) => {
			const providers = [...row.providers];
			const target = index + direction;
			if (target < 0 || target >= providers.length) return row;
			[providers[index], providers[target]] = [providers[target], providers[index]];
			return { ...row, providers };
		});
	}

	function removeProvider(field: MetadataField, providerId: string): void {
		updateRow(field, (row) => ({
			...row,
			providers: row.providers.filter((id) => id !== providerId)
		}));
	}

	function addProvider(field: MetadataField, providerId: string | undefined): void {
		if (!providerId) return;
		updateRow(field, (row) =>
			row.providers.includes(providerId)
				? row
				: { ...row, providers: [...row.providers, providerId] }
		);
	}

	function setStrategy(field: MetadataField, strategy: string | undefined): void {
		if (!strategy) return;
		updateRow(field, (row) => ({ ...row, strategy: strategy as MetadataPolicyStrategy }));
	}

	function setLockRespected(field: MetadataField, lockRespected: boolean): void {
		updateRow(field, (row) => ({ ...row, lockRespected }));
	}

	// Drop a field from the override so it inherits the server default again.
	// The row is only removed from the saved payload; the server answers with
	// the inherited rule.
	function inheritField(field: MetadataField): void {
		policyRows = policyRows.map((row) =>
			row.field === field ? { ...row, overridden: false } : row
		);
		policyDirty = policyDirty.filter((dirty) => dirty !== field);
	}

	// The override document: every field the library already overrides plus
	// every field edited in this session. Fields left inherited stay out of
	// it, so the server default keeps flowing through.
	function overrideRows(): PolicyRow[] {
		return policyRows.filter((row) => row.overridden || policyDirty.includes(row.field));
	}

	function availableProviders(row: PolicyRow): string[] {
		return policyProviderIds.filter((id) => !row.providers.includes(id));
	}

	// The audio section of the override: sent when the library already
	// overrides it or it was edited here, `null` otherwise so the server
	// default keeps flowing through.
	function overrideAudio(): AudioPolicy | null {
		if (!audioPolicy) return null;
		return audioPolicy.overridden || audioDirty ? audioPolicy : null;
	}

	function updateAudio(change: Partial<AudioPolicy>): void {
		if (!audioPolicy) return;
		audioPolicy = { ...audioPolicy, ...change };
		audioDirty = true;
	}

	function inheritAudio(): void {
		if (!audioPolicy) return;
		audioPolicy = { ...audioPolicy, overridden: false };
		audioDirty = false;
	}

	// `keepOriginal: false` is only meaningful with `autoAssemble` on — the
	// server refuses the pair — so turning assemble off restores it rather
	// than sending a document that would be rejected.
	function setAutoAssemble(autoAssemble: boolean): void {
		updateAudio(autoAssemble ? { autoAssemble } : { autoAssemble, keepOriginal: true });
	}

	function switchChecked(event: Event): boolean {
		return (event.currentTarget as HTMLButtonElement).getAttribute('data-state') === 'checked';
	}
</script>

<svelte:head><title>Settings · Coppice ingest</title></svelte:head>

<div class="flex flex-col gap-6">
	<div>
		<p class="text-sm font-medium text-primary">Registry and policy</p>
		<h1 class="text-3xl font-semibold tracking-tight">Ingest settings</h1>
		<p class="mt-1 max-w-3xl text-muted-foreground">Configure metadata providers, the per-field metadata policy, and deterministic quality checks. Secret values are write-only and never rendered back from the server.</p>
	</div>
	<Tabs.Root bind:value={tab}>
		<Tabs.List class="h-auto w-fit"><Tabs.Trigger value="providers">Metadata providers</Tabs.Trigger><Tabs.Trigger value="policy">Policy</Tabs.Trigger><Tabs.Trigger value="quality">Quality checks</Tabs.Trigger></Tabs.List>
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
		<Tabs.Content value="policy" class="flex flex-col gap-6">
			<Card>
				<CardHeader>
					<div class="flex flex-wrap items-start justify-between gap-4">
						<div>
							<CardTitle>Per-field metadata policy</CardTitle>
							<CardDescription>
								For every field: which providers may fill it, in what order, and how their values combine. A field left inherited follows the server default; the library override only carries the fields you change.
							</CardDescription>
						</div>
						<Badge variant={policy?.hasLibraryOverride ? 'secondary' : 'outline'}>
							{policy?.hasLibraryOverride ? 'Library override' : 'Server default'}
						</Badge>
					</div>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					<div class="grid max-w-xl gap-2 text-sm font-medium">
						<span>Library</span>
						<Select.Root type="single" name="policy-library" bind:value={selectedLibraryId} onValueChange={selectLibrary}>
							<Select.Trigger class="w-full">
								{libraries.find((library) => library.id === selectedLibraryId)?.name ?? 'Select a library'}
							</Select.Trigger>
							<Select.Content>
								<Select.Group>
									<Select.Label>Libraries</Select.Label>
									{#each libraries as library (library.id)}
										<Select.Item value={library.id} label={library.name}>{library.name}</Select.Item>
									{/each}
								</Select.Group>
							</Select.Content>
						</Select.Root>
					</div>
					{#if policyQuery.isPending}
						<div class="flex flex-col gap-3"><Skeleton class="h-12 w-full" /><Skeleton class="h-12 w-full" /><Skeleton class="h-12 w-full" /></div>
					{:else if policyQuery.isError}
						<Alert variant="destructive">
							<AlertTitle>Unable to load the metadata policy</AlertTitle>
							<AlertDescription>{policyQuery.error instanceof Error ? policyQuery.error.message : 'The server did not return a policy.'}</AlertDescription>
						</Alert>
					{:else if !policyRows.length}
						<Empty>
							<EmptyHeader>
								<EmptyTitle>No policy fields</EmptyTitle>
								<EmptyDescription>Select a library to load its effective metadata policy.</EmptyDescription>
							</EmptyHeader>
						</Empty>
					{:else}
						<div class="flex flex-col divide-y">
							{#each policyRows as row (row.field)}
								<div class="grid gap-4 py-4 lg:grid-cols-[14rem_1fr_15rem]">
									<div class="flex flex-col gap-1">
										<span class="font-medium">{fieldLabel(row.field)}</span>
										<span class="text-xs text-muted-foreground">{row.field}</span>
										<div class="flex flex-wrap gap-1">
											{#if row.overridden || policyDirty.includes(row.field)}
												<Badge variant="secondary" class="text-xs">Override</Badge>
											{:else}
												<Badge variant="outline" class="text-xs">Inherited</Badge>
											{/if}
											{#if !row.storable}
												<Badge variant="outline" class="text-xs" title="The staged apply path has no column for this field: the winner is shown as evidence, nothing is written.">Evidence only</Badge>
											{/if}
										</div>
									</div>
									<div class="flex flex-col gap-2">
										{#if row.providers.length}
											<ol class="flex flex-col gap-1">
												{#each row.providers as providerId, index (providerId)}
													<li class="flex items-center gap-2 text-sm">
														<span class="w-5 tabular-nums text-muted-foreground">{index + 1}.</span>
														<span class="min-w-0 flex-1 truncate">{providers.find((provider) => provider.id === providerId)?.name ?? providerId}</span>
														<span class="truncate text-xs text-muted-foreground">{providerId}</span>
														<Button type="button" variant="ghost" size="sm" aria-label={`Move ${providerId} up`} disabled={index === 0} onclick={() => moveProvider(row.field, index, -1)}>↑</Button>
														<Button type="button" variant="ghost" size="sm" aria-label={`Move ${providerId} down`} disabled={index === row.providers.length - 1} onclick={() => moveProvider(row.field, index, 1)}>↓</Button>
														<Button type="button" variant="ghost" size="sm" aria-label={`Remove ${providerId}`} onclick={() => removeProvider(row.field, providerId)}>✕</Button>
													</li>
												{/each}
											</ol>
										{:else}
											<p class="text-sm text-muted-foreground">No provider may fill this field.</p>
										{/if}
										{#if availableProviders(row).length}
											<Select.Root type="single" name={`add-provider-${row.field}`} value="" onValueChange={(providerId) => addProvider(row.field, providerId)}>
												<Select.Trigger class="w-full max-w-sm">Add provider</Select.Trigger>
												<Select.Content>
													<Select.Group>
														<Select.Label>Registered providers</Select.Label>
														{#each availableProviders(row) as providerId (providerId)}
															<Select.Item value={providerId} label={providerId}>
																{providers.find((provider) => provider.id === providerId)?.name ?? providerId} · {providerId}
															</Select.Item>
														{/each}
													</Select.Group>
												</Select.Content>
											</Select.Root>
										{/if}
									</div>
									<div class="flex flex-col gap-2">
										<Select.Root type="single" name={`strategy-${row.field}`} value={row.strategy} onValueChange={(strategy) => setStrategy(row.field, strategy)}>
											<Select.Trigger class="w-full">{strategyLabel(row.strategy)}</Select.Trigger>
											<Select.Content>
												<Select.Group>
													<Select.Label>Merge strategy</Select.Label>
													{#each STRATEGIES as strategy (strategy.value)}
														<Select.Item value={strategy.value} label={strategy.label}>{strategy.label}</Select.Item>
													{/each}
												</Select.Group>
											</Select.Content>
										</Select.Root>
										<p class="text-xs text-muted-foreground">{STRATEGIES.find((entry) => entry.value === row.strategy)?.hint}</p>
										<div class="flex items-center justify-between gap-2">
											<label class="flex items-center gap-2 text-sm" for={`lock-${row.field}`}>
												Respect locks
												<Switch id={`lock-${row.field}`} checked={row.lockRespected} onchange={(event) => setLockRespected(row.field, (event.currentTarget as HTMLButtonElement).getAttribute('data-state') === 'checked')} />
											</label>
											{#if row.overridden || policyDirty.includes(row.field)}
												<Button type="button" variant="ghost" size="sm" onclick={() => inheritField(row.field)}>Inherit</Button>
											{/if}
										</div>
									</div>
								</div>
							{/each}
						</div>
						{#if audioPolicy}
							{@const audio = audioPolicy}
							<div class="flex flex-col gap-4 border-t pt-4">
								<div class="flex flex-wrap items-center gap-2">
									<span class="font-medium">Audio</span>
									{#if audio.overridden || audioDirty}
										<Badge variant="secondary" class="text-xs">Override</Badge>
									{:else}
										<Badge variant="outline" class="text-xs">Inherited</Badge>
									{/if}
									{#if audio.overridden || audioDirty}
										<Button type="button" variant="ghost" size="sm" onclick={inheritAudio}>Inherit</Button>
									{/if}
								</div>
								<p class="text-sm text-muted-foreground">
									Audiobook ingest for this library: how heavily a book split across several files
									counts against its quality score, and whether ingest may rewrite it.
								</p>
								<div class="grid gap-4 lg:grid-cols-[20rem_1fr]">
									<div class="flex flex-col gap-1">
										<label class="text-sm font-medium" for="audio-single-file-weight">Split-book penalty</label>
										<Input
											id="audio-single-file-weight"
											type="number"
											min="0"
											max="100"
											value={audio.singleFileWeight}
											oninput={(event) =>
												updateAudio({
													singleFileWeight: Math.max(
														0,
														Math.min(100, Number.parseInt((event.currentTarget as HTMLInputElement).value, 10) || 0)
													)
												})}
										/>
										<p class="text-xs text-muted-foreground">
											Weight of the <code>single_file</code> check, on the same 0–100 scale as every
											other check. 0 keeps the finding advisory without moving the score.
										</p>
									</div>
									<div class="flex flex-col gap-3">
										<label class="flex items-center justify-between gap-2 text-sm" for="audio-auto-assemble">
											<span>
												Assemble split books
												<span class="block text-xs text-muted-foreground">Merge a multi-file audiobook into one canonical M4B during ingest.</span>
											</span>
											<Switch id="audio-auto-assemble" checked={audio.autoAssemble} onchange={(event) => setAutoAssemble(switchChecked(event))} />
										</label>
										<label class="flex items-center justify-between gap-2 text-sm" for="audio-auto-chapters">
											<span>
												Write missing chapters
												<span class="block text-xs text-muted-foreground">Derive chapter marks from file boundaries when a book has none.</span>
											</span>
											<Switch id="audio-auto-chapters" checked={audio.autoChapters} onchange={(event) => updateAudio({ autoChapters: switchChecked(event) })} />
										</label>
										<label class="flex items-center justify-between gap-2 text-sm" for="audio-keep-original">
											<span>
												Keep source files
												<span class="block text-xs text-muted-foreground">
													{audio.autoAssemble
														? 'Turn off to delete the source files once an assemble succeeds.'
														: 'Only applies when “Assemble split books” is on.'}
												</span>
											</span>
											<Switch
												id="audio-keep-original"
												checked={audio.keepOriginal}
												disabled={!audio.autoAssemble}
												onchange={(event) => updateAudio({ keepOriginal: switchChecked(event) })}
											/>
										</label>
									</div>
								</div>
							</div>
						{/if}
						<div class="flex flex-wrap items-center justify-end gap-2">
							<Button type="button" variant="outline" disabled={savePolicyMutation.isPending || !policy?.hasLibraryOverride} onclick={() => savePolicyMutation.mutate({ fields: [], audio: null })}>
								Reset to server default
							</Button>
							<Button type="button" disabled={savePolicyMutation.isPending || !policyChanged} onclick={() => savePolicyMutation.mutate({ fields: overrideRows(), audio: overrideAudio() })}>
								{savePolicyMutation.isPending ? 'Saving…' : 'Save policy'}
							</Button>
						</div>
					{/if}
				</CardContent>
			</Card>
		</Tabs.Content>
		<Tabs.Content value="quality" class="flex flex-col gap-4">
			{#if qualityCatalogQuery.isPending}{#each Array(5) as _, index (index)}<Skeleton class="h-24 w-full" />{/each}{:else if qualityCatalogQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load quality checks</AlertTitle><AlertDescription>{qualityCatalogQuery.error instanceof Error ? qualityCatalogQuery.error.message : 'The server did not return quality descriptors.'}</AlertDescription></Alert>{:else if !checks.length}<Empty><EmptyHeader><EmptyTitle>No quality checks registered</EmptyTitle><EmptyDescription>The server build has no ingest quality checks available.</EmptyDescription></EmptyHeader></Empty>{:else}{#each checks as check (check.id)}<Card><CardContent class="flex flex-wrap items-start justify-between gap-4 p-5"><div><div class="flex items-center gap-2"><h2 class="font-medium">{check.name}</h2><span class="rounded bg-muted px-2 py-0.5 text-xs tabular-nums">weight {check.weight}</span></div><p class="mt-1 text-sm text-muted-foreground">{check.id} · version {check.version} · {check.supportedMediaTypes.join(', ') || 'all formats'}</p>{#if check.settings.length}<p class="mt-2 text-xs text-muted-foreground">Settings: {check.settings.map((setting) => setting.label).join(', ')}</p>{/if}</div><label class="flex items-center gap-2 text-sm">Enabled<Switch checked={check.enabled} onchange={(event) => saveQualityMutation.mutate({ checkId: check.id, enabled: (event.currentTarget as HTMLButtonElement).getAttribute('data-state') === 'checked' })} /></label></CardContent></Card>{/each}{/if}
		</Tabs.Content>
	</Tabs.Root>
	<Separator />
	<p class="text-sm text-muted-foreground">Changing a check setting invalidates affected reports and requeues items; the server preserves prior algorithm-versioned evidence.</p>
</div>

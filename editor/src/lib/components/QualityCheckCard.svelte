<script lang="ts">
	import { browser } from '$app/environment';
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import * as Select from '@stump/ui/components/ui/select';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { Textarea } from '@stump/ui/components/ui/textarea';
	import { request } from '@stump/ui/graphql/client';
	import {
		IngestQualityCheckSettingsDocument,
		SetIngestQualityCheckSettingsDocument,
		type IngestQualityCheckCatalogQuery
	} from '$lib/graphql/generated/graphql';
	import {
		boundsLabel,
		draftFromValue,
		validateSetting,
		type SettingCheck,
		type SettingDescriptor,
		type SettingDraft
	} from '$lib/ingest/settings';

	type QualityCheck = IngestQualityCheckCatalogQuery['ingestQualityCheckCatalog'][number];

	let { check, canManage }: { check: QualityCheck; canManage: boolean } = $props();

	const queryClient = useQueryClient();
	const catalogKey = ['quality-catalog'];

	// `enabled` has its own switch in the header and is stored in the row's
	// enabled column; every other non-secret descriptor is a tunable setting.
	let definitions = $derived<SettingDescriptor[]>(
		check.settings.filter((definition) => definition.key !== 'enabled' && !definition.secret)
	);
	let settingsKey = $derived(['quality-settings', check.id]);

	const settingsQuery = createQuery(() => ({
		queryKey: settingsKey,
		queryFn: () => request(IngestQualityCheckSettingsDocument, { checkId: check.id }),
		enabled: browser && definitions.length > 0
	}));
	let stored = $derived(settingsQuery.data?.ingestQualityCheckSettings);

	// What the server holds: a draft per setting, `null` where the check runs
	// on its default. `drafts` is a writable derived keyed on the serialized
	// baseline, so local edits survive a catalog refresh (e.g. toggling the
	// check) and are replaced only when the stored values actually change.
	let baseline = $derived.by((): Record<string, SettingDraft> => {
		const values = stored?.settings ?? [];
		return Object.fromEntries(
			definitions.map((definition) => {
				const value = values.find((setting) => setting.key === definition.key)?.value;
				return [definition.key, value === null || value === undefined ? null : draftFromValue(definition, value)];
			})
		);
	});
	let baselineSnapshot = $derived(JSON.stringify(baseline));
	let drafts = $derived(JSON.parse(baselineSnapshot) as Record<string, SettingDraft>);
	let results = $derived.by((): Record<string, SettingCheck | null> =>
		Object.fromEntries(
			definitions.map((definition) => {
				const draft = drafts[definition.key];
				return [definition.key, draft === null || draft === undefined ? null : validateSetting(definition, draft)];
			})
		)
	);
	let invalid = $derived(Object.values(results).some((result) => result !== null && !result.ok));
	let dirty = $derived(definitions.some((definition) => drafts[definition.key] !== baseline[definition.key]));

	const enabledMutation = createMutation(() => ({
		mutationFn: (enabled: boolean) =>
			request(SetIngestQualityCheckSettingsDocument, { input: { checkId: check.id, enabled } }),
		onSuccess: (result) => {
			const saved = result.setIngestQualityCheckSettings;
			queryClient.setQueryData(settingsKey, { ingestQualityCheckSettings: saved });
			queryClient.setQueryData<IngestQualityCheckCatalogQuery>(catalogKey, (catalog) =>
				catalog && {
					ingestQualityCheckCatalog: catalog.ingestQualityCheckCatalog.map((entry) =>
						entry.id === saved.checkId ? { ...entry, enabled: saved.enabled } : entry
					)
				}
			);
			toast.success(`${check.name} ${saved.enabled ? 'enabled' : 'disabled'}.`);
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to change the quality check.')
	}));

	const saveMutation = createMutation(() => ({
		mutationFn: (settings: Record<string, unknown>) =>
			request(SetIngestQualityCheckSettingsDocument, { input: { checkId: check.id, settings } }),
		onSuccess: (result) => {
			queryClient.setQueryData(settingsKey, {
				ingestQualityCheckSettings: result.setIngestQualityCheckSettings
			});
			void queryClient.invalidateQueries({ queryKey: catalogKey });
			toast.success(`${check.name} settings saved.`);
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to save quality-check settings.')
	}));

	function defaultDraft(definition: SettingDescriptor): string | boolean {
		return draftFromValue(definition, definition.defaultValue);
	}

	function current(definition: SettingDescriptor): string | boolean {
		return drafts[definition.key] ?? defaultDraft(definition);
	}

	function setDraft(key: string, value: SettingDraft): void {
		drafts = { ...drafts, [key]: value };
	}

	function defaultLabel(definition: SettingDescriptor): string {
		const value = defaultDraft(definition);
		if (typeof value === 'boolean') return value ? 'on' : 'off';
		return value === '' ? 'empty' : value;
	}

	// Inherited settings are left out, so the server keeps following the
	// check's default; the saved object replaces the stored one.
	function save(event: SubmitEvent): void {
		event.preventDefault();
		if (invalid || !dirty) return;
		const settings: Record<string, unknown> = {};
		for (const definition of definitions) {
			const result = results[definition.key];
			if (result?.ok) settings[definition.key] = result.value;
		}
		saveMutation.mutate(settings);
	}
</script>

<Card>
	<CardContent class="flex flex-col gap-4 p-5">
		<div class="flex flex-wrap items-start justify-between gap-4">
			<div>
				<div class="flex items-center gap-2">
					<h2 class="font-medium">{check.name}</h2>
					<span class="rounded bg-muted px-2 py-0.5 text-xs tabular-nums">weight {check.weight}</span>
				</div>
				<p class="mt-1 text-sm text-muted-foreground">
					{check.id} · version {check.version} · {check.supportedMediaTypes.join(', ') || 'all formats'}
				</p>
			</div>
			<label class="flex items-center gap-2 text-sm">
				Enabled
				<Switch
					bind:checked={() => check.enabled, (enabled) => enabledMutation.mutate(enabled)}
					disabled={!canManage || enabledMutation.isPending}
				/>
			</label>
		</div>

		{#if definitions.length}
			{#if settingsQuery.isPending}
				<Skeleton class="h-16 w-full" />
			{:else if settingsQuery.isError}
				<Alert variant="destructive">
					<AlertTitle>Unable to load settings</AlertTitle>
					<AlertDescription>
						{settingsQuery.error instanceof Error ? settingsQuery.error.message : 'The server did not return stored settings.'}
					</AlertDescription>
				</Alert>
			{:else}
				<form class="flex flex-col gap-4 border-t pt-4" novalidate onsubmit={save}>
					<div class="grid gap-5 md:grid-cols-2">
						{#each definitions as definition (definition.key)}
							{@const id = `quality-${check.id}-${definition.key}`}
							{@const result = results[definition.key]}
							{@const error = result && !result.ok ? result.error : null}
							{@const inherited = drafts[definition.key] === null}
							{@const bounds = boundsLabel(definition)}
							<div class="grid content-start gap-2 text-sm">
								<div class="flex flex-wrap items-center justify-between gap-2">
									<label class="font-medium" for={id}>{definition.label}</label>
									{#if inherited}
										<Badge variant="secondary">Default</Badge>
									{:else}
										<Button
											type="button"
											variant="ghost"
											size="sm"
											disabled={!canManage}
											onclick={() => setDraft(definition.key, null)}
										>
											Reset to default ({defaultLabel(definition)})
										</Button>
									{/if}
								</div>
								{#if definition.valueType === 'BOOLEAN'}
									<Switch
										{id}
										bind:checked={() => current(definition) === true, (value) => setDraft(definition.key, value)}
										disabled={!canManage}
									/>
								{:else if definition.valueType === 'INTEGER' || definition.valueType === 'NUMBER'}
									<Input
										{id}
										type="number"
										inputmode={definition.valueType === 'INTEGER' ? 'numeric' : 'decimal'}
										step={definition.valueType === 'INTEGER' ? 1 : 'any'}
										min={definition.minimum ?? undefined}
										max={definition.maximum ?? undefined}
										value={current(definition)}
										oninput={(event) => setDraft(definition.key, event.currentTarget.value)}
										aria-invalid={error ? 'true' : undefined}
										aria-describedby={`${id}-hint`}
										disabled={!canManage}
									/>
								{:else if definition.valueType === 'ENUM'}
									{#if definition.options.length}
										<Select.Root
											type="single"
											name={id}
											value={String(current(definition))}
											onValueChange={(value) => setDraft(definition.key, value)}
											disabled={!canManage}
										>
											<Select.Trigger {id} class="w-full" aria-invalid={error ? 'true' : undefined}>
												{String(current(definition)) || 'Choose an option'}
											</Select.Trigger>
											<Select.Content>
												<Select.Group>
													<Select.Label>{definition.label}</Select.Label>
													{#each definition.options as option (option)}
														<Select.Item value={option} label={option}>{option}</Select.Item>
													{/each}
												</Select.Group>
											</Select.Content>
										</Select.Root>
									{:else}
										<p {id} class="text-xs text-destructive">
											The server lists no options for this setting, so it cannot be edited here.
										</p>
									{/if}
								{:else if definition.valueType === 'JSON'}
									<Textarea
										{id}
										class="font-mono text-xs"
										value={String(current(definition))}
										oninput={(event) => setDraft(definition.key, event.currentTarget.value)}
										aria-invalid={error ? 'true' : undefined}
										aria-describedby={`${id}-hint`}
										disabled={!canManage}
									/>
								{:else}
									<Input
										{id}
										value={String(current(definition))}
										oninput={(event) => setDraft(definition.key, event.currentTarget.value)}
										aria-invalid={error ? 'true' : undefined}
										aria-describedby={`${id}-hint`}
										required={definition.required}
										disabled={!canManage}
									/>
								{/if}
								<p id={`${id}-hint`} class={['text-xs', error ? 'text-destructive' : 'text-muted-foreground']}>
									{#if error}
										{error}
									{:else}
										{definition.description ?? `Expected ${definition.valueType.toLowerCase()} value.`}{bounds ? ` Allowed: ${bounds}.` : ''}
									{/if}
								</p>
							</div>
						{/each}
					</div>
					<div class="flex flex-wrap items-center justify-end gap-2">
						{#if !canManage}
							<p class="mr-auto text-xs text-muted-foreground">
								Changing quality checks needs the metadata-provider and library management permissions.
							</p>
						{/if}
						<Button type="button" variant="outline" disabled={!dirty || saveMutation.isPending} onclick={() => (drafts = baseline)}>
							Discard changes
						</Button>
						<Button type="submit" disabled={!canManage || !dirty || invalid || saveMutation.isPending}>
							{saveMutation.isPending ? 'Saving…' : 'Save settings'}
						</Button>
					</div>
				</form>
			{/if}
		{/if}
	</CardContent>
</Card>

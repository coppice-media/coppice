<script lang="ts">
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import * as Dialog from '@stump/ui/components/ui/dialog';
	import { Input } from '@stump/ui/components/ui/input';
	import * as Select from '@stump/ui/components/ui/select';
	import { request } from '@stump/ui/graphql/client';
	import {
		RenameDeviceDocument,
		RevokeDeviceDocument,
		RotateDeviceCredentialDocument,
		SetDeviceTransformProfileDocument
	} from '$lib/graphql/generated/graphql';
	import {
		DEVICE_KIND_LABELS,
		NO_PRESET,
		TRANSFORMABLE_KINDS,
		TRANSFORM_PRESETS,
		presetOf,
		type Device,
		type IssuedCredential
	} from '$lib/devices';
	import { absoluteTime, relativeTime, summarizeSync } from '$lib/format';
	import CredentialReveal from './CredentialReveal.svelte';

	let { device, now }: { device: Device; now: Date } = $props();

	const queryClient = useQueryClient();
	const revoked = $derived(device.revokedAt !== null && device.revokedAt !== undefined);
	const preset = $derived(presetOf(device.transformProfile));
	const presetLabel = $derived(
		TRANSFORM_PRESETS.find((candidate) => candidate.name === preset)?.label ??
			(preset === NO_PRESET ? 'Server default' : `Custom (${preset})`)
	);
	const syncSummary = $derived(summarizeSync(device.lastSyncSummary));

	let renaming = $state(false);
	let draftName = $state('');
	let rotated = $state<IssuedCredential | null>(null);
	let rotatedOpen = $state(false);

	function invalidate(): void {
		void queryClient.invalidateQueries({ queryKey: ['devices'] });
	}

	function failure(fallback: string): (error: unknown) => void {
		return (error) => toast.error(error instanceof Error ? error.message : fallback);
	}

	const rename = createMutation(() => ({
		mutationFn: (name: string) => request(RenameDeviceDocument, { id: device.id, name }),
		onSuccess: () => {
			renaming = false;
			invalidate();
		},
		onError: failure('Unable to rename the device.')
	}));
	const rotate = createMutation(() => ({
		mutationFn: () => request(RotateDeviceCredentialDocument, { id: device.id }),
		onSuccess: (result) => {
			rotated = result.rotateDeviceCredential;
			rotatedOpen = true;
			invalidate();
		},
		onError: failure('Unable to rotate the credential.')
	}));
	const revoke = createMutation(() => ({
		mutationFn: () => request(RevokeDeviceDocument, { id: device.id }),
		onSuccess: () => {
			toast.success(`${device.name} can no longer sign in.`);
			invalidate();
		},
		onError: failure('Unable to revoke the device.')
	}));
	const setPreset = createMutation(() => ({
		mutationFn: (name: string) =>
			request(SetDeviceTransformProfileDocument, {
				id: device.id,
				profile: name === NO_PRESET ? null : { preset: name }
			}),
		onSuccess: () => {
			toast.success(`Transform preset saved for ${device.name}.`);
			invalidate();
		},
		onError: failure('Unable to save the transform preset.')
	}));

	function startRename(): void {
		draftName = device.name;
		renaming = true;
	}

	function submitRename(event: SubmitEvent): void {
		event.preventDefault();
		const name = draftName.trim();
		if (!name || name === device.name) {
			renaming = false;
			return;
		}
		rename.mutate(name);
	}

	function confirmRevoke(): void {
		if (window.confirm(`Revoke ${device.name}? Its credential stops working immediately.`)) {
			revoke.mutate();
		}
	}

	function confirmRotate(): void {
		if (
			window.confirm(
				`Rotate the credential for ${device.name}? The current secret stops working and a new one is shown once.`
			)
		) {
			rotate.mutate();
		}
	}
</script>

<Card class={revoked ? 'opacity-70' : ''} data-device-id={device.id}>
	<CardHeader>
		<div class="flex flex-wrap items-start gap-2">
			<div class="min-w-0 flex-1">
				{#if renaming}
					<form class="flex items-center gap-2" onsubmit={submitRename}>
						<Input
							bind:value={draftName}
							maxlength={100}
							aria-label="Device name"
							autofocus
							class="h-8"
						/>
						<Button type="submit" size="sm" disabled={rename.isPending}>Save</Button>
						<Button type="button" size="sm" variant="ghost" onclick={() => (renaming = false)}>
							Cancel
						</Button>
					</form>
				{:else}
					<CardTitle class="truncate text-base">{device.name}</CardTitle>
				{/if}
			</div>
			<Badge variant="secondary">{DEVICE_KIND_LABELS[device.kind]}</Badge>
			{#if revoked}
				<Badge variant="destructive">Revoked</Badge>
			{:else if device.credential}
				<Badge variant="outline">{device.credential.protocol}</Badge>
			{/if}
		</div>
	</CardHeader>
	<CardContent class="grid gap-x-6 gap-y-1 text-sm sm:grid-cols-2">
		<div class="flex justify-between gap-2 sm:block">
			<span class="text-muted-foreground">Last seen</span>
			<span class="sm:block" title={absoluteTime(device.lastSeenAt)}>
				{relativeTime(device.lastSeenAt, now)}
			</span>
		</div>
		<div class="flex justify-between gap-2 sm:block">
			<span class="text-muted-foreground">Last sync</span>
			<span class="sm:block" title={absoluteTime(device.lastSyncAt)}>
				{relativeTime(device.lastSyncAt, now)}
			</span>
		</div>
		<div class="flex justify-between gap-2 sm:block">
			<span class="text-muted-foreground">Credential</span>
			<span class="font-mono text-xs sm:block">
				{device.credential ? device.credential.secretHint : 'none'}
			</span>
		</div>
		<div class="flex justify-between gap-2 sm:block">
			<span class="text-muted-foreground">Registered</span>
			<span class="sm:block" title={absoluteTime(device.createdAt)}>
				{relativeTime(device.createdAt, now)}
			</span>
		</div>
		{#if syncSummary}
			<p class="text-xs text-muted-foreground sm:col-span-2">{syncSummary}</p>
		{/if}
		{#if TRANSFORMABLE_KINDS[device.kind] && !revoked}
			<div class="mt-2 flex flex-col gap-1 sm:col-span-2">
				<span class="text-muted-foreground">Comic transform preset</span>
				<Select.Root
					type="single"
					value={preset}
					onValueChange={(value) => setPreset.mutate(value)}
					disabled={setPreset.isPending}
				>
					<Select.Trigger class="w-full sm:max-w-sm" aria-label="Comic transform preset">
						{presetLabel}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value={NO_PRESET} label="Server default" />
						<Select.Group>
							<Select.Label>Presets</Select.Label>
							{#each TRANSFORM_PRESETS as candidate (candidate.name)}
								<Select.Item value={candidate.name} label={candidate.label} />
							{/each}
						</Select.Group>
					</Select.Content>
				</Select.Root>
			</div>
		{/if}
	</CardContent>
	{#if !revoked}
		<CardFooter class="flex flex-wrap gap-2">
			<Button size="sm" variant="outline" onclick={startRename} disabled={renaming}>Rename</Button>
			{#if device.credential}
				<Button size="sm" variant="outline" onclick={confirmRotate} disabled={rotate.isPending}>
					{rotate.isPending ? 'Rotating…' : 'Rotate credential'}
				</Button>
			{/if}
			<Button
				size="sm"
				variant="destructive"
				class="ml-auto"
				onclick={confirmRevoke}
				disabled={revoke.isPending}
			>
				Revoke
			</Button>
		</CardFooter>
	{/if}
</Card>

<Dialog.Root bind:open={rotatedOpen}>
	<Dialog.Content class="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>New credential for {device.name}</Dialog.Title>
			<Dialog.Description>The previous secret no longer works.</Dialog.Description>
		</Dialog.Header>
		{#if rotated}
			<div class="min-h-0 flex-1 overflow-y-auto pr-1">
				<CredentialReveal issued={rotated} />
			</div>
		{/if}
		<Dialog.Footer>
			<Button onclick={() => (rotatedOpen = false)}>Done</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

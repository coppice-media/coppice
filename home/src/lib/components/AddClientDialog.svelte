<script lang="ts">
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Button } from '@stump/ui/components/ui/button';
	import * as Dialog from '@stump/ui/components/ui/dialog';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { request } from '@stump/ui/graphql/client';
	import { CreateDeviceDocument, type DeviceKind } from '$lib/graphql/generated/graphql';
	import { ADDABLE_KINDS, DEVICE_KIND_LABELS, type IssuedCredential } from '$lib/devices';
	import CredentialReveal from './CredentialReveal.svelte';

	let { open = $bindable(false) }: { open?: boolean } = $props();

	let kind = $state<DeviceKind>('KOBO');
	let name = $state('');
	let issued = $state<IssuedCredential | null>(null);

	const queryClient = useQueryClient();
	const createMutationState = createMutation(() => ({
		mutationFn: () =>
			request(CreateDeviceDocument, { kind, name: name.trim() ? name.trim() : null }),
		onSuccess: (result) => {
			issued = result.createDevice;
			void queryClient.invalidateQueries({ queryKey: ['devices'] });
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to register the client.')
	}));

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		createMutationState.mutate();
	}

	function reset(): void {
		kind = 'KOBO';
		name = '';
		issued = null;
	}

	$effect(() => {
		if (!open) reset();
	});
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>{issued ? `${issued.device.name} is registered` : 'Add a client'}</Dialog.Title>
			<Dialog.Description>
				{#if issued}
					Point the {DEVICE_KIND_LABELS[issued.device.kind]} at the endpoints below.
				{:else}
					Register a reader or integration. Stump mints a credential for it and shows where to
					point it.
				{/if}
			</Dialog.Description>
		</Dialog.Header>

		{#if issued}
			<div class="min-h-0 flex-1 overflow-y-auto pr-1">
				<CredentialReveal {issued} />
			</div>
			<Dialog.Footer>
				<Button onclick={() => (open = false)}>Done</Button>
			</Dialog.Footer>
		{:else}
			<form class="flex flex-col gap-5" onsubmit={submit}>
				<fieldset class="flex flex-col gap-2">
					<legend class="mb-2 text-sm font-medium">Client kind</legend>
					<div class="grid gap-2 sm:grid-cols-2">
						{#each ADDABLE_KINDS as option (option.kind)}
							<label
								class="flex cursor-pointer items-start gap-3 rounded-lg border p-3 text-sm has-[:checked]:border-primary has-[:checked]:bg-muted/60"
							>
								<input
									type="radio"
									name="kind"
									value={option.kind}
									bind:group={kind}
									class="mt-1"
								/>
								<span class="flex flex-col gap-0.5">
									<span class="font-medium">{DEVICE_KIND_LABELS[option.kind]}</span>
									<span class="text-xs text-muted-foreground">{option.description}</span>
								</span>
							</label>
						{/each}
					</div>
				</fieldset>
				<div class="flex flex-col gap-2">
					<Label for="device-name">Name</Label>
					<Input
						id="device-name"
						bind:value={name}
						placeholder={`My ${DEVICE_KIND_LABELS[kind]}`}
						maxlength={100}
					/>
					<p class="text-xs text-muted-foreground">Optional; Stump picks a default when empty.</p>
				</div>
				<Dialog.Footer>
					<Button type="button" variant="outline" onclick={() => (open = false)}>Cancel</Button>
					<Button type="submit" disabled={createMutationState.isPending}>
						{createMutationState.isPending ? 'Registering…' : 'Register client'}
					</Button>
				</Dialog.Footer>
			</form>
		{/if}
	</Dialog.Content>
</Dialog.Root>

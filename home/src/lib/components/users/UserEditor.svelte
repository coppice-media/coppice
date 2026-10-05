<script lang="ts">
	/**
	 * The create/edit form for one account, rendered inside the `/users` sheet.
	 * Creating sends `createUser`; editing sends `updateUser`, which replaces
	 * the stored permissions, age restriction, and session limit as a whole,
	 * so every one of them is sent back even when only one changed.
	 */
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { untrack } from 'svelte';
	import { toast } from 'svelte-sonner';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import ShieldAlertIcon from '@lucide/svelte/icons/shield-alert';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Checkbox } from '@stump/ui/components/ui/checkbox';
	import { Input } from '@stump/ui/components/ui/input';
	import * as InputGroup from '@stump/ui/components/ui/input-group';
	import { Label } from '@stump/ui/components/ui/label';
	import { Separator } from '@stump/ui/components/ui/separator';
	import * as Sheet from '@stump/ui/components/ui/sheet';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { request } from '@stump/ui/graphql/client';
	import { errorMessage } from '@stump/ui/utils/errors.js';
	import {
		UsersCreateDocument,
		UsersUpdateDocument,
		type UserPermission
	} from '$lib/graphql/generated/graphql';
	import {
		OIDC_PERMISSION_SYNC_DOCS,
		explicitPermissions,
		storedPermissions,
		type ManagedUser
	} from '$lib/users';
	import PermissionPicker from './PermissionPicker.svelte';

	let {
		user,
		oidcEnabled,
		onsaved
	}: {
		/** The account to edit, or `null` to create one. */
		user: ManagedUser | null;
		oidcEnabled: boolean;
		onsaved: () => void;
	} = $props();

	// The sheet mounts this form each time it opens, so the stored values are
	// only its starting point: a list refetch while it is open must not
	// overwrite edits in progress.
	const initial = untrack(() => ({
		id: user?.id ?? null,
		username: user?.username ?? '',
		oidcLinked: Boolean(user?.oidcEmail),
		granted: explicitPermissions(user?.permissions ?? []),
		age: user?.ageRestriction?.age ?? null,
		restrictOnUnset: user?.ageRestriction?.restrictOnUnset ?? false,
		sessionLimit: user?.maxSessionsAllowed ?? null
	}));

	let username = $state(initial.username);
	let password = $state('');
	let showPassword = $state(false);
	let granted = $state<UserPermission[]>(initial.granted);
	let restrictAge = $state(initial.age !== null);
	let age = $state<number | null>(initial.age);
	let restrictOnUnset = $state(initial.restrictOnUnset);
	let sessionLimit = $state<number | null>(initial.sessionLimit);

	const ageValid = $derived(!restrictAge || (age !== null && Number.isInteger(age) && age >= 0));
	const sessionLimitValid = $derived(
		sessionLimit === null || (Number.isInteger(sessionLimit) && sessionLimit >= 1)
	);
	const canSubmit = $derived(
		username.trim().length > 0 &&
			(initial.id !== null || password.length > 0) &&
			ageValid &&
			sessionLimitValid
	);

	const queryClient = useQueryClient();
	const save = createMutation(() => ({
		mutationFn: async (): Promise<string> => {
			const fields = {
				permissions: storedPermissions(granted),
				ageRestriction: restrictAge && age !== null ? { age, restrictOnUnset } : null,
				maxSessionsAllowed: sessionLimit
			};
			if (initial.id === null) {
				const result = await request(UsersCreateDocument, {
					input: { ...fields, username: username.trim(), password }
				});
				return result.createUser.username;
			}
			// `username` is required input; sending the stored one back means a
			// save never renames the account.
			const result = await request(UsersUpdateDocument, {
				id: initial.id,
				input: { ...fields, username: initial.username, password: null }
			});
			return result.updateUser.username;
		},
		onSuccess: (name) => {
			toast.success(
				initial.id === null
					? `Created ${name}.`
					: `Saved ${name}. Their sessions ended, so the changes apply when they next sign in.`
			);
			void queryClient.invalidateQueries({ queryKey: ['users'] });
			onsaved();
		}
	}));

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		if (canSubmit && !save.isPending) save.mutate();
	}
</script>

<Sheet.Header class="border-b pr-12">
	<Sheet.Title>{initial.id === null ? 'New user' : `Edit ${initial.username}`}</Sheet.Title>
	<Sheet.Description>
		{initial.id === null
			? 'A local account that signs in with this username and password. It starts with only the permissions you tick.'
			: 'Saving replaces this account’s permissions, age restriction, and session limit, and signs it out everywhere so the changes apply at its next sign-in.'}
	</Sheet.Description>
</Sheet.Header>

<form class="flex min-h-0 flex-1 flex-col" onsubmit={submit}>
	<div class="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto overscroll-contain px-4 py-5">
		{#if initial.id === null}
			<section class="flex flex-col gap-4" aria-label="Sign-in">
				<div class="flex flex-col gap-2">
					<Label for="user-username">Username</Label>
					<Input id="user-username" bind:value={username} autocomplete="off" required />
				</div>
				<div class="flex flex-col gap-2">
					<Label for="user-password">Password</Label>
					<InputGroup.Root>
						<InputGroup.Input
							id="user-password"
							type={showPassword ? 'text' : 'password'}
							bind:value={password}
							autocomplete="new-password"
							required
						/>
						<InputGroup.Addon align="inline-end">
							<InputGroup.Button
								size="icon-xs"
								aria-label={showPassword ? 'Hide password' : 'Show password'}
								aria-pressed={showPassword}
								onclick={() => (showPassword = !showPassword)}
							>
								{#if showPassword}
									<EyeOffIcon aria-hidden="true" />
								{:else}
									<EyeIcon aria-hidden="true" />
								{/if}
							</InputGroup.Button>
						</InputGroup.Addon>
					</InputGroup.Root>
					<p class="text-xs text-muted-foreground">
						Share it with them directly. Whether they may change it is the
						<em>Change own password</em> permission below.
					</p>
				</div>
			</section>
			<Separator />
		{/if}

		<section class="flex flex-col gap-3" aria-labelledby="user-permissions-heading">
			<div class="flex flex-col gap-1">
				<h3 id="user-permissions-heading" class="text-sm font-medium">Permissions</h3>
				<p class="text-xs text-muted-foreground">
					Ticking a permission also grants what it includes; those show as included.
				</p>
			</div>
			{#if initial.oidcLinked && oidcEnabled}
				<Alert role="note">
					<ShieldAlertIcon aria-hidden="true" />
					<AlertTitle>Their next sign-in may replace these</AlertTitle>
					<AlertDescription>
						<p>
							{initial.username} signs in through OIDC. While OIDC permission sync is on
							(<code>STUMP_OIDC_SYNC_PERMISSIONS</code>, the default) and provider groups are
							mapped, every sign-in replaces their permissions with their groups’ permissions.
							Coppice does not tell this screen whether sync is on.
							<a
								href={OIDC_PERMISSION_SYNC_DOCS}
								target="_blank"
								rel="noreferrer"
								class="text-primary underline-offset-4 hover:underline"
							>
								How OIDC sync works
							</a>
						</p>
					</AlertDescription>
				</Alert>
			{/if}
			<PermissionPicker bind:granted disabled={save.isPending} />
		</section>

		<Separator />

		<section class="flex flex-col gap-3" aria-labelledby="user-age-heading">
			<div class="flex items-start justify-between gap-4">
				<div class="flex flex-col gap-1">
					<h3 id="user-age-heading" class="text-sm font-medium">Age restriction</h3>
					<p class="text-xs text-muted-foreground">
						Hide books rated above an age. Ratings come from book or series metadata.
					</p>
				</div>
				<Switch
					bind:checked={restrictAge}
					aria-labelledby="user-age-heading"
					disabled={save.isPending}
				/>
			</div>
			{#if restrictAge}
				<div class="flex flex-col gap-2">
					<Label for="user-age">Highest rating they may read</Label>
					<Input
						id="user-age"
						type="number"
						min="0"
						step="1"
						inputmode="numeric"
						class="w-32"
						bind:value={age}
						aria-invalid={!ageValid || undefined}
						aria-describedby="user-age-hint"
						required
					/>
					<p
						id="user-age-hint"
						class={['text-xs', ageValid ? 'text-muted-foreground' : 'text-destructive']}
					>
						{ageValid
							? 'A book rated 13 is hidden from a reader limited to 12.'
							: 'Enter a whole number, 0 or more.'}
					</p>
				</div>
				<div class="flex items-start gap-3">
					<Checkbox
						id="user-age-unset"
						class="mt-0.5"
						bind:checked={restrictOnUnset}
						aria-describedby="user-age-unset-hint"
					/>
					<div class="flex flex-col gap-1">
						<Label for="user-age-unset" class="font-normal">Also hide books without a rating</Label>
						<p id="user-age-unset-hint" class="text-xs text-muted-foreground">
							Otherwise a book with no rating on it or its series stays visible.
						</p>
					</div>
				</div>
			{/if}
		</section>

		<Separator />

		<section class="flex flex-col gap-2">
			<Label for="user-session-limit">Session limit</Label>
			<Input
				id="user-session-limit"
				type="number"
				min="1"
				step="1"
				inputmode="numeric"
				class="w-32"
				placeholder="No limit"
				bind:value={sessionLimit}
				aria-invalid={!sessionLimitValid || undefined}
				aria-describedby="user-session-hint"
			/>
			<p
				id="user-session-hint"
				class={['text-xs', sessionLimitValid ? 'text-muted-foreground' : 'text-destructive']}
			>
				{sessionLimitValid
					? 'How many sessions may be signed in at once; at the limit, signing in again ends the oldest. Leave it empty for no limit.'
					: 'Enter a whole number, 1 or more, or leave it empty.'}
			</p>
		</section>
	</div>

	<Sheet.Footer class="border-t">
		{#if save.isError}
			<Alert variant="destructive">
				<AlertTitle>
					{initial.id === null ? 'The account was not created' : 'The changes were not saved'}
				</AlertTitle>
				<AlertDescription>{errorMessage(save.error)}</AlertDescription>
			</Alert>
		{/if}
		<div class="flex justify-end gap-2">
			<Sheet.Close>
				{#snippet child({ props })}
					<Button {...props} type="button" variant="outline">Cancel</Button>
				{/snippet}
			</Sheet.Close>
			<Button type="submit" disabled={!canSubmit || save.isPending}>
				{#if save.isPending}
					{initial.id === null ? 'Creating…' : 'Saving…'}
				{:else}
					{initial.id === null ? 'Create user' : 'Save changes'}
				{/if}
			</Button>
		</div>
	</Sheet.Footer>
</form>

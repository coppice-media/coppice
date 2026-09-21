<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createQuery } from '@tanstack/svelte-query';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Separator } from '@stump/ui/components/ui/separator';

	interface OidcConfig {
		enabled: boolean;
		allowRegistration: boolean;
		disableLocalAuth: boolean;
	}

	let username = $state('');
	let password = $state('');
	let submitting = $state(false);
	let errorMessage = $state<string | null>(null);
	const returnTo = $derived.by(() => {
		const candidate = page.url.searchParams.get('returnTo');
		return isSafeReturnTo(candidate) ? candidate : resolve('/dashboard');
	});
	const manual = $derived(page.url.searchParams.get('manual') === '1');
	const hasOidcError = $derived(page.url.searchParams.has('error'));
	const oidcError = $derived(page.url.searchParams.get('error'));
	const displayedError = $derived(
		errorMessage ?? oidcError ?? (hasOidcError ? 'Single sign-on sign-in failed.' : null)
	);

	function isSafeReturnTo(value: string | null): value is string {
		return (
			value !== null &&
			value.startsWith('/') &&
			!value.startsWith('//') &&
			!value.includes('://') &&
			!value.includes('\\') &&
			!(/[\u0000-\u001f\u007f]/.test(value))
		);
	}

	const oidcQuery = createQuery(() => ({
		queryKey: ['oidcConfig'],
		queryFn: async (): Promise<OidcConfig> => {
			const response = await fetch('/api/v2/auth/oidc/config', { credentials: 'include' });
			if (!response.ok) return { enabled: false, allowRegistration: false, disableLocalAuth: false };
			return (await response.json()) as OidcConfig;
		},
		enabled: browser,
		staleTime: Number.POSITIVE_INFINITY
	}));
	const oidc = $derived(oidcQuery.data);

	$effect(() => {
		if (!browser || !oidc?.enabled || hasOidcError || (!oidc.disableLocalAuth && manual)) return;
		window.location.assign(
			`/api/v2/auth/oidc/authorize?return_to=${encodeURIComponent(returnTo)}`
		);
	});

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		errorMessage = null;
		submitting = true;
		try {
			const response = await fetch('/api/v2/auth/login', {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				credentials: 'include',
				body: JSON.stringify({ username, password })
			});
			if (!response.ok) {
				let message = 'The username or password was not accepted.';
				try {
					const payload = (await response.json()) as { message?: string; error?: string };
					message = payload.message ?? payload.error ?? message;
				} catch {
					// Keep the user-facing authentication message when the server has no JSON body.
				}
				throw new Error(message);
			}
			window.location.href = returnTo;
		} catch (error) {
			errorMessage = error instanceof Error ? error.message : 'Unable to sign in.';
			submitting = false;
		}
	}

	function signInWithOidc(): void {
		window.location.assign(
			`/api/v2/auth/oidc/authorize?return_to=${encodeURIComponent(returnTo)}`
		);
	}

</script>

<svelte:head>
	<title>Sign in · Coppice</title>
</svelte:head>

<div class="flex min-h-screen items-center justify-center bg-muted/30 px-4 py-12">
	<Card class="w-full max-w-md">
		<CardHeader>
			<CardTitle>Sign in to Coppice</CardTitle>
			<CardDescription>Manage your devices, reading activity, and account.</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-col gap-5">
			{#if displayedError}
				<p class="text-sm text-destructive" role="alert">{displayedError}</p>
			{/if}
			{#if !oidc?.disableLocalAuth}
				<form class="flex flex-col gap-5" onsubmit={submit}>
					<div class="flex flex-col gap-2">
						<Label for="username">Username</Label>
						<Input id="username" autocomplete="username" bind:value={username} required />
					</div>
					<div class="flex flex-col gap-2">
						<Label for="password">Password</Label>
						<Input
							id="password"
							type="password"
							autocomplete="current-password"
							bind:value={password}
							required
						/>
					</div>
					<Button type="submit" disabled={submitting}>
						{submitting ? 'Signing in…' : 'Sign in'}
					</Button>
				</form>
			{/if}
			{#if oidc?.enabled}
				{#if !oidc.disableLocalAuth}
					<div class="flex items-center gap-3 text-xs text-muted-foreground">
						<Separator class="flex-1" />
						or
						<Separator class="flex-1" />
					</div>
				{/if}
				<Button variant="outline" onclick={signInWithOidc}>Sign in with single sign-on</Button>
			{/if}
		</CardContent>
	</Card>
</div>

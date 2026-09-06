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
	let returnTo = $derived(page.url.searchParams.get('returnTo') ?? resolve('/dashboard'));

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
			window.location.href = returnTo.startsWith('/') ? returnTo : resolve('/dashboard');
		} catch (error) {
			errorMessage = error instanceof Error ? error.message : 'Unable to sign in.';
			submitting = false;
		}
	}

	// The server creates the session in its callback and redirects to `/`;
	// the web UI (or a 404 on headless builds) takes it from there.
	function signInWithOidc(): void {
		window.location.assign('/api/v2/auth/oidc/authorize');
	}
</script>

<svelte:head>
	<title>Sign in · Stump</title>
</svelte:head>

<div class="flex min-h-screen items-center justify-center bg-muted/30 px-4 py-12">
	<Card class="w-full max-w-md">
		<CardHeader>
			<CardTitle>Sign in to Stump</CardTitle>
			<CardDescription>Manage your devices, reading activity, and account.</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-col gap-5">
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
					{#if errorMessage}
						<p class="text-sm text-destructive" role="alert">{errorMessage}</p>
					{/if}
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

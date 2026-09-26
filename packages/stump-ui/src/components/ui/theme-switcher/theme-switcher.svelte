<script lang="ts">
	/**
	 * Shared appearance controls, sized for a sidebar rail or a header row:
	 * the colour mode is an icon toggle with tooltips and the palette grid
	 * sits behind a popover, so neither the Home rail nor the editor header
	 * has to fit sixteen swatches and three labelled buttons inline.
	 */
	import MonitorIcon from "@lucide/svelte/icons/monitor";
	import MoonIcon from "@lucide/svelte/icons/moon";
	import SunIcon from "@lucide/svelte/icons/sun";
	import { buttonVariants } from "@stump/ui/components/ui/button/index.js";
	import * as Popover from "@stump/ui/components/ui/popover/index.js";
	import * as ToggleGroup from "@stump/ui/components/ui/toggle-group/index.js";
	import * as Tooltip from "@stump/ui/components/ui/tooltip/index.js";
	import { cn } from "@stump/ui/utils.js";
	import { uuid } from "@stump/ui/utils/uuid.js";
	import { THEME_PRESETS, theme, type ThemeMode } from "@stump/ui/theme.svelte.js";

	let { class: className }: { class?: string } = $props();

	const MODES: { value: ThemeMode; label: string; icon: typeof SunIcon }[] = [
		{ value: "dark", label: "Dark", icon: MoonIcon },
		{ value: "light", label: "Light", icon: SunIcon },
		{ value: "system", label: "System", icon: MonitorIcon }
	];

	// Several switchers can be mounted at once (the rail and the mobile
	// sheet); radio groups only behave when each has its own name.
	const group = `theme-preset-${uuid()}`;

	const current = $derived(THEME_PRESETS.find((preset) => preset.id === theme.preset) ?? THEME_PRESETS[0]);
</script>

<div data-slot="theme-switcher" class={cn("flex w-fit max-w-full items-center gap-2", className)}>
	<ToggleGroup.Root
		type="single"
		variant="outline"
		size="sm"
		value={theme.mode}
		onValueChange={(value) => {
			if (value) theme.mode = value as ThemeMode;
		}}
		aria-label="Colour mode"
	>
		{#each MODES as mode (mode.value)}
			{@const Icon = mode.icon}
			<Tooltip.Root>
				<Tooltip.Trigger>
					{#snippet child({ props })}
						<ToggleGroup.Item {...props} value={mode.value} aria-label={mode.label} class="size-8">
							<Icon aria-hidden="true" />
						</ToggleGroup.Item>
					{/snippet}
				</Tooltip.Trigger>
				<Tooltip.Content>{mode.label}</Tooltip.Content>
			</Tooltip.Root>
		{/each}
	</ToggleGroup.Root>

	<Popover.Root>
		<Tooltip.Root>
			<Tooltip.Trigger>
				{#snippet child({ props: tooltipProps })}
					<Popover.Trigger
						{...tooltipProps}
						aria-label={`Palette: ${current.label}`}
						class={cn(buttonVariants({ variant: "outline", size: "icon-sm" }))}
					>
						<span
							aria-hidden="true"
							style={`background: ${current.swatch}`}
							class="block size-4 rounded-full border border-foreground/25"
						></span>
					</Popover.Trigger>
				{/snippet}
			</Tooltip.Trigger>
			<Tooltip.Content>Palette: {current.label}</Tooltip.Content>
		</Tooltip.Root>
		<Popover.Content align="end" class="w-64 gap-2 p-3">
			<fieldset data-slot="theme-switcher-presets">
				<legend class="mb-2 text-xs font-medium text-muted-foreground">Palette</legend>
				<div class="grid grid-cols-8 gap-1">
					{#each THEME_PRESETS as preset (preset.id)}
						<label
							class={cn(
								"group/preset relative flex min-w-0 cursor-pointer items-center justify-center rounded-md border border-transparent p-1 transition-colors hover:bg-accent/70",
								preset.kind === "special" && "border-border/70 bg-muted/40"
							)}
							title={`${preset.label}: ${preset.description}`}
						>
							<input
								type="radio"
								name={group}
								value={preset.id}
								checked={theme.preset === preset.id}
								aria-label={`${preset.label} palette — ${preset.description}`}
								onchange={() => (theme.preset = preset.id)}
								class="peer sr-only"
							/>
							<span
								aria-hidden="true"
								style={`background: ${preset.swatch}`}
								class={cn(
									"block size-5 rounded-full border border-foreground/20 ring-ring/60 ring-offset-2 ring-offset-background transition-[border-color,box-shadow,transform]",
									"peer-checked:scale-110 peer-checked:ring-2 peer-focus-visible:ring-2 peer-focus-visible:ring-ring group-hover/preset:border-foreground/45",
									preset.kind === "special" && "rounded-md border-foreground/30"
								)}
							></span>
							<span class="sr-only">{preset.label}</span>
						</label>
					{/each}
				</div>
			</fieldset>
			<p class="text-xs text-muted-foreground">{current.label} · {current.description}</p>
		</Popover.Content>
	</Popover.Root>
</div>

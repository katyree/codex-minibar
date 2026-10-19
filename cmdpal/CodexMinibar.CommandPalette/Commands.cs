using System.IO;
using System.Threading.Tasks;
using CodexMinibar.CommandPalette.Bridge;
using Microsoft.CommandPalette.Extensions;
using Microsoft.CommandPalette.Extensions.Toolkit;

namespace CodexMinibar.CommandPalette;

internal static class Icons
{
    public static IconInfo App { get; } = IconHelpers.FromRelativePath("Assets\\StoreLogo.png");

    // Segoe Fluent Icons glyphs.
    public static IconInfo Open { get; } = new("");

    public static IconInfo Refresh { get; } = new("");

    public static IconInfo Settings { get; } = new("");

    public static IconInfo Warning { get; } = new("");

    /// <summary>A provider mark for each theme, or the app icon without one.</summary>
    public static IconInfo From(IconPair? pair)
    {
        if (pair is null || !File.Exists(pair.Light) || !File.Exists(pair.Dark))
        {
            return App;
        }

        return new IconInfo(new IconData(pair.Light), new IconData(pair.Dark));
    }
}

/// <summary>Opens the Minibar popup on a provider, starting Minibar if needed.</summary>
internal sealed partial class OpenPopupCommand : InvokableCommand
{
    private readonly string? _provider;

    public OpenPopupCommand(string? provider)
    {
        _provider = provider;
        Id = $"codexminibar.open.{provider ?? "popup"}";
        Name = "Open in Minibar";
        Icon = Icons.Open;
    }

    public override ICommandResult Invoke()
    {
        _ = Task.Run(async () =>
        {
            try
            {
                await MinibarBridge.OpenPopupAsync(_provider).ConfigureAwait(false);
            }
            catch (BridgeUnavailableException)
            {
                MinibarBridge.Launch();
            }
        });
        return CommandResult.Dismiss();
    }
}

/// <summary>Asks Minibar to fetch fresh limits, then polls it again.</summary>
internal sealed partial class RefreshCommand : InvokableCommand
{
    private readonly MinibarState _state;

    public RefreshCommand(MinibarState state)
    {
        _state = state;
        Id = "codexminibar.refresh";
        Name = "Refresh limits";
        Icon = Icons.Refresh;
    }

    public override ICommandResult Invoke()
    {
        _ = Task.Run(async () =>
        {
            try
            {
                await MinibarBridge.RefreshAsync().ConfigureAwait(false);

                // The refresh runs in the background; give it a moment.
                await Task.Delay(3000).ConfigureAwait(false);
            }
            catch (BridgeUnavailableException)
            {
                // The poll below reports it.
            }

            _state.Poke();
        });
        return CommandResult.KeepOpen();
    }
}

/// <summary>Opens Minibar Settings where the Dock bands are configured.</summary>
internal sealed partial class OpenSettingsCommand : InvokableCommand
{
    public OpenSettingsCommand()
    {
        Id = "codexminibar.settings";
        Name = "Configure bands in Minibar";
        Icon = Icons.Settings;
    }

    public override ICommandResult Invoke()
    {
        _ = Task.Run(async () =>
        {
            try
            {
                await MinibarBridge.OpenSettingsAsync().ConfigureAwait(false);
            }
            catch (BridgeUnavailableException)
            {
                MinibarBridge.Launch();
            }
        });
        return CommandResult.Dismiss();
    }
}

/// <summary>Starts Minibar when it is not running.</summary>
internal sealed partial class LaunchCommand : InvokableCommand
{
    private readonly MinibarState _state;

    public LaunchCommand(MinibarState state)
    {
        _state = state;
        Id = "codexminibar.launch";
        Name = "Start Codex Minibar";
        Icon = Icons.App;
    }

    public override ICommandResult Invoke()
    {
        if (!MinibarBridge.Launch())
        {
            return CommandResult.ShowToast("Codex Minibar is not installed");
        }

        _ = Task.Run(async () =>
        {
            await Task.Delay(4000).ConfigureAwait(false);
            _state.Poke();
        });
        return CommandResult.Dismiss();
    }
}

using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace CodexMinibar.CommandPalette.Bridge;

/// <summary>
/// The latest palette payload, polled from Minibar while Command Palette keeps
/// the extension alive. Raises <see cref="Changed"/> after every poll.
/// </summary>
public sealed partial class MinibarState : IDisposable
{
    private static readonly TimeSpan PollInterval = TimeSpan.FromSeconds(15);

    private readonly CancellationTokenSource _stop = new();
    private readonly SemaphoreSlim _polling = new(1, 1);
    private readonly Lock _lock = new();
    private List<Band> _bands;
    private List<ProviderSummary> _providers = [];
    private string? _error = "Codex Minibar is not running";

    public MinibarState()
    {
        _bands = LoadCache();

        // The host asks for commands and bands right after activation; answer
        // with live values when Minibar is up instead of an empty first frame.
        try
        {
            PollAsync().Wait(TimeSpan.FromSeconds(2));
        }
        catch (AggregateException)
        {
            // PollAsync records its own failures.
        }

        _ = Task.Run(LoopAsync);
    }

    public event EventHandler? Changed;

    /// <summary>Null while Minibar answers; otherwise why it does not.</summary>
    public string? Error
    {
        get
        {
            lock (_lock)
            {
                return _error;
            }
        }
    }

    /// <summary>Live bands, or the ones seen last while Minibar is closed.</summary>
    public IReadOnlyList<Band> Bands
    {
        get
        {
            lock (_lock)
            {
                return _bands;
            }
        }
    }

    public IReadOnlyList<ProviderSummary> Providers
    {
        get
        {
            lock (_lock)
            {
                return _providers;
            }
        }
    }

    private static string CachePath => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "CodexMinibar.CommandPalette",
        "bands.json");

    /// <summary>Polls now instead of waiting for the next tick.</summary>
    public void Poke() => _ = Task.Run(PollAsync);

    public void Dispose()
    {
        _stop.Cancel();
        _stop.Dispose();
    }

    private async Task LoopAsync()
    {
        using var timer = new PeriodicTimer(PollInterval);
        try
        {
            while (await timer.WaitForNextTickAsync(_stop.Token).ConfigureAwait(false))
            {
                await PollAsync().ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException)
        {
            // Disposed.
        }
    }

    private async Task PollAsync()
    {
        if (!await _polling.WaitAsync(0).ConfigureAwait(false))
        {
            return;
        }

        try
        {
            var reply = await MinibarBridge.PaletteAsync().ConfigureAwait(false);
            bool bandsChanged;
            lock (_lock)
            {
                bandsChanged = !SameBands(_bands, reply.Bands);
                _bands = reply.Bands;
                _providers = reply.Providers;
                _error = null;
            }

            if (bandsChanged)
            {
                SaveCache(reply.Bands);
            }
        }
        catch (Exception error)
        {
            lock (_lock)
            {
                _providers = [];
                _error = MinibarBridge.Describe(error);
            }
        }
        finally
        {
            _polling.Release();
        }

        Changed?.Invoke(this, EventArgs.Empty);
    }

    private static bool SameBands(List<Band> left, List<Band> right)
    {
        if (left.Count != right.Count)
        {
            return false;
        }

        for (var i = 0; i < left.Count; i++)
        {
            if (left[i].Id != right[i].Id || left[i].Name != right[i].Name || left[i].Provider != right[i].Provider)
            {
                return false;
            }
        }

        return true;
    }

    private static List<Band> LoadCache()
    {
        try
        {
            var cache = JsonSerializer.Deserialize(File.ReadAllText(CachePath), BridgeJson.Default.BandCache);
            return cache?.Bands ?? [];
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or JsonException)
        {
            return [];
        }
    }

    private static void SaveCache(List<Band> bands)
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(CachePath)!);
            File.WriteAllText(CachePath, JsonSerializer.Serialize(new BandCache { Bands = bands }, BridgeJson.Default.BandCache));
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
        {
            // The cache only bridges a closed Minibar; losing it is harmless.
        }
    }
}

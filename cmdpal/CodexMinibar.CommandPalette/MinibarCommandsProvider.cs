using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using CodexMinibar.CommandPalette.Bridge;
using Microsoft.CommandPalette.Extensions;
using Microsoft.CommandPalette.Extensions.Toolkit;

namespace CodexMinibar.CommandPalette;

/// <summary>
/// Top-level commands (the summary plus one per provider, so typing a
/// provider's name shows its limits) and the Dock bands configured in Minibar.
/// Items are updated in place so the Dock repaints without re-pinning.
/// </summary>
public sealed partial class MinibarCommandsProvider : CommandProvider
{
    private const string BandPrefix = "codexminibar.band.";
    private const string NotRunning = "Minibar is not running";

    private readonly Lock _lock = new();
    private readonly MinibarState _state = new();
    private readonly SummaryPage _summary;
    private readonly CommandItem _summaryItem;
    private readonly Dictionary<string, ProviderPage> _pages = [];
    private readonly Dictionary<string, (CommandItem Item, string? Icon)> _providerItems = [];
    private readonly Dictionary<string, (WrappedDockItem Band, ListItem Item, string? Provider, string? Icon)> _bands = [];
    private ICommandItem[] _topLevel = [];
    private ICommandItem[] _dockBands = [];

    public MinibarCommandsProvider()
    {
        Id = "CodexMinibar";
        DisplayName = "Codex Minibar";
        Icon = Icons.App;
        Frozen = false;

        _summary = new SummaryPage(_state, Page);
        _summaryItem = new CommandItem(_summary)
        {
            Title = "Codex Minibar",
            Subtitle = "AI usage limits",
            Icon = Icons.App,
            MoreCommands = Rows.Actions(_state, null),
        };

        Sync();
        _state.Changed += OnChanged;
    }

    public override ICommandItem[] TopLevelCommands()
    {
        lock (_lock)
        {
            return _topLevel;
        }
    }

    public override ICommandItem[] GetDockBands()
    {
        lock (_lock)
        {
            return _dockBands;
        }
    }

    public override ICommandItem? GetCommandItem(string id)
    {
        lock (_lock)
        {
            return _topLevel.Concat(_dockBands).FirstOrDefault(item => item.Command?.Id == id);
        }
    }

    public override void Dispose()
    {
        _state.Changed -= OnChanged;
        _state.Dispose();
        _summary.Dispose();
        lock (_lock)
        {
            foreach (var page in _pages.Values)
            {
                page.Dispose();
            }
        }

        base.Dispose();
    }

    private ProviderPage Page(ProviderSummary provider)
    {
        lock (_lock)
        {
            if (!_pages.TryGetValue(provider.Id, out var page))
            {
                page = new ProviderPage(_state, provider.Id, provider.Name);
                _pages[provider.Id] = page;
            }

            return page;
        }
    }

    private void OnChanged(object? sender, EventArgs e)
    {
        if (Sync())
        {
            RaiseItemsChanged();
        }
    }

    /// <summary>Applies the latest state; true when the item sets changed.</summary>
    private bool Sync()
    {
        var providers = _state.Providers;
        var bands = _state.Bands;
        var running = _state.Error is null;
        lock (_lock)
        {
            var changed = SyncProviders(providers);
            changed |= SyncBands(bands, running);
            return changed;
        }
    }

    private bool SyncProviders(IReadOnlyList<ProviderSummary> providers)
    {
        var changed = !_topLevel.Skip(1).Select(item => item.Command?.Id)
            .SequenceEqual(providers.Select(provider => ProviderPage.CommandId(provider.Id)));
        var items = new List<ICommandItem> { _summaryItem };
        var seen = new HashSet<string>();
        foreach (var provider in providers)
        {
            seen.Add(provider.Id);
            var icon = provider.Icon?.Light;
            if (!_providerItems.TryGetValue(provider.Id, out var entry))
            {
                entry = (new CommandItem(Page(provider)) { MoreCommands = Rows.Actions(_state, provider.Id) }, null);
            }

            entry.Item.Title = provider.Name;
            entry.Item.Subtitle = provider.Headline;
            if (entry.Icon != icon || entry.Item.Icon is null)
            {
                entry.Item.Icon = Icons.From(provider.Icon);
            }

            _providerItems[provider.Id] = (entry.Item, icon);
            items.Add(entry.Item);
        }

        foreach (var id in _providerItems.Keys.Where(id => !seen.Contains(id)).ToList())
        {
            _providerItems.Remove(id);
        }

        _topLevel = [.. items];
        return changed;
    }

    private bool SyncBands(IReadOnlyList<Band> bands, bool running)
    {
        var changed = !_dockBands.Select(item => item.Command?.Id)
            .SequenceEqual(bands.Select(band => BandPrefix + band.Id));
        var items = new List<ICommandItem>();
        var seen = new HashSet<string>();
        foreach (var band in bands)
        {
            seen.Add(band.Id);
            var icon = band.Icon?.Light;
            // The Dock lists a band by its wrapper title and opens its
            // provider; neither can change in place, so rebuild the band.
            if (!_bands.TryGetValue(band.Id, out var entry)
                || entry.Band.Title != band.Name
                || entry.Provider != band.Provider)
            {
                changed = true;
                var item = new ListItem(new OpenPopupCommand(band.Provider))
                {
                    MoreCommands =
                    [
                        new CommandContextItem(new RefreshCommand(_state)),
                        new CommandContextItem(new OpenSettingsCommand()),
                    ],
                };
                entry = (new WrappedDockItem([item], BandPrefix + band.Id, band.Name), item, band.Provider, null);
            }

            // The cached band keeps its name and icon while Minibar is closed.
            entry.Item.Title = running ? band.Title : band.Name;
            entry.Item.Subtitle = running ? band.Subtitle : NotRunning;
            if (entry.Icon != icon || entry.Item.Icon is null)
            {
                entry.Item.Icon = Icons.From(band.Icon);
            }

            _bands[band.Id] = (entry.Band, entry.Item, entry.Provider, icon);
            items.Add(entry.Band);
        }

        foreach (var id in _bands.Keys.Where(id => !seen.Contains(id)).ToList())
        {
            _bands.Remove(id);
        }

        _dockBands = [.. items];
        return changed;
    }
}

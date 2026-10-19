using System;
using System.Collections.Generic;
using System.Linq;
using CodexMinibar.CommandPalette.Bridge;
using Microsoft.CommandPalette.Extensions;
using Microsoft.CommandPalette.Extensions.Toolkit;

namespace CodexMinibar.CommandPalette;

internal static class Rows
{
    private const string Separator = " · ";

    /// <summary>`46% left · resets in 2h 5m`.</summary>
    public static string QuotaLine(Quota quota) => quota.ResetIn is { Length: > 0 } reset
        ? $"{quota.ValueText}{Separator}resets in {reset}"
        : quota.ValueText;

    public static Details ProviderDetails(ProviderSummary provider)
    {
        var metadata = new List<IDetailsElement>();
        if (provider.Plan is { Length: > 0 } plan)
        {
            metadata.Add(new DetailsElement { Key = "Plan", Data = new DetailsLink { Text = plan } });
        }

        foreach (var quota in provider.Quotas)
        {
            var text = quota.ResetAt is { Length: > 0 } at
                ? $"{QuotaLine(quota)} ({at})"
                : QuotaLine(quota);
            metadata.Add(new DetailsElement { Key = quota.Label, Data = new DetailsLink { Text = text } });
        }

        if (provider.Today is { Length: > 0 } today)
        {
            metadata.Add(new DetailsElement { Key = "Today", Data = new DetailsLink { Text = today } });
        }

        return new Details
        {
            Title = provider.Name,
            HeroImage = Icons.From(provider.Icon),
            Metadata = [.. metadata],
        };
    }

    public static ITag[] PlanTags(ProviderSummary provider) =>
        provider.Plan is { Length: > 0 } plan ? [new Tag(plan)] : [];

    public static IContextItem[] Actions(MinibarState state, string? provider) =>
    [
        new CommandContextItem(new RefreshCommand(state)),
        new CommandContextItem(new OpenSettingsCommand()),
        new CommandContextItem(new OpenPopupCommand(provider)),
    ];

    /// <summary>The single row shown while Minibar does not answer.</summary>
    public static ListItem Unavailable(MinibarState state) => new(new LaunchCommand(state))
    {
        Title = "Codex Minibar is not running",
        Subtitle = state.Error is { Length: > 0 } error && error != "Codex Minibar is not running"
            ? error
            : "Start it to see your limits",
        Icon = Icons.Warning,
    };
}

/// <summary>Every enabled provider with its quotas; typed as "minibar".</summary>
internal sealed partial class SummaryPage : ListPage, IDisposable
{
    private readonly MinibarState _state;
    private readonly Func<ProviderSummary, ProviderPage> _page;

    public SummaryPage(MinibarState state, Func<ProviderSummary, ProviderPage> page)
    {
        _state = state;
        _page = page;
        Id = "codexminibar.summary";
        Name = "Open";
        Title = "Codex Minibar";
        Icon = Icons.App;
        PlaceholderText = "Search providers";
        ShowDetails = true;
        _state.Changed += OnChanged;
    }

    public override IListItem[] GetItems()
    {
        var providers = _state.Providers;
        if (_state.Error is not null)
        {
            return [Rows.Unavailable(_state)];
        }

        return [.. providers.Select(provider => new ListItem(_page(provider))
        {
            Title = provider.Name,
            Subtitle = provider.Headline,
            Icon = Icons.From(provider.Icon),
            Tags = Rows.PlanTags(provider),
            Details = Rows.ProviderDetails(provider),
            MoreCommands = Rows.Actions(_state, provider.Id),
        })];
    }

    public void Dispose() => _state.Changed -= OnChanged;

    private void OnChanged(object? sender, EventArgs e) => RaiseItemsChanged();
}

/// <summary>One provider's quotas, one row each.</summary>
internal sealed partial class ProviderPage : ListPage, IDisposable
{
    private readonly MinibarState _state;
    private readonly string _provider;

    public ProviderPage(MinibarState state, string provider, string name)
    {
        _state = state;
        _provider = provider;
        Id = CommandId(provider);
        Name = "Show limits";
        Title = name;
        Icon = Icons.App;
        _state.Changed += OnChanged;
    }

    public static string CommandId(string provider) => $"codexminibar.provider.{provider}";

    public override IListItem[] GetItems()
    {
        if (_state.Error is not null)
        {
            return [Rows.Unavailable(_state)];
        }

        var provider =_state.Providers.FirstOrDefault(provider => provider.Id == _provider);
        if (provider is null)
        {
            return [];
        }

        var open = new OpenPopupCommand(provider.Id);
        var icon = Icons.From(provider.Icon);
        var items = provider.Quotas.Select(quota => new ListItem(open)
        {
            Title = quota.Label,
            Subtitle = Rows.QuotaLine(quota),
            Icon = icon,
            // A plain amount (OpenRouter's balance) already is the subtitle.
            Tags = quota.Value == quota.ValueText ? [] : [new Tag(quota.Value)],
            MoreCommands = Rows.Actions(_state, provider.Id),
        }).ToList();
        if (provider.Today is { Length: > 0 } today)
        {
            items.Add(new ListItem(open) { Title = "Today", Subtitle = today, Icon = icon });
        }

        return [.. items];
    }

    public void Dispose() => _state.Changed -= OnChanged;

    private void OnChanged(object? sender, EventArgs e) => RaiseItemsChanged();
}

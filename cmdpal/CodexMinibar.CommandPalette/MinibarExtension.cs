using System;
using System.Runtime.InteropServices;
using System.Threading;
using Microsoft.CommandPalette.Extensions;

namespace CodexMinibar.CommandPalette;

/// <summary>The COM class the manifest registers; hands out the provider.</summary>
[Guid("DDCFDC6F-533A-4144-BBFF-2792EEBFBD81")]
public sealed partial class MinibarExtension : IExtension, IDisposable
{
    private readonly ManualResetEvent _disposed;
    private readonly MinibarCommandsProvider _provider = new();

    public MinibarExtension(ManualResetEvent disposed)
    {
        _disposed = disposed;
    }

    public object? GetProvider(ProviderType providerType) => providerType switch
    {
        ProviderType.Commands => _provider,
        _ => null,
    };

    public void Dispose()
    {
        _provider.Dispose();
        _disposed.Set();
    }
}

using System;
using System.Threading;
using Microsoft.CommandPalette.Extensions;
using Shmuelie.WinRTServer.CsWinRT;

namespace CodexMinibar.CommandPalette;

public static class Program
{
    [MTAThread]
    public static void Main(string[] args)
    {
        if (args.Length == 0 || args[0] != "-RegisterProcessAsComServer")
        {
            Console.WriteLine("Command Palette starts this extension; it does nothing on its own.");
            return;
        }

        global::Shmuelie.WinRTServer.ComServer server = new();
        using ManualResetEvent disposed = new(false);

        // One instance for every request, so the host and the Dock share the
        // same live state, and the process exits once the host disposes it.
        MinibarExtension extension = new(disposed);
        server.RegisterClass<MinibarExtension, IExtension>(() => extension);
        server.Start();
        disposed.WaitOne();
        server.Stop();
        server.UnsafeDispose();
    }
}

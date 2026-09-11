using System.ComponentModel;
using System.Text.Json;
using Gmail.Console.Infrastructure;
using Gmail.Console.Mail;
using Spectre.Console;
using Spectre.Console.Cli;

namespace Gmail.Console.Commands.Thread;

public sealed class GetCommand : MailboxCommand<GetCommand.Settings>
{
    public sealed class Settings : AccountSettings
    {
        [CommandArgument(0, "<THREAD-ID>")]
        public string ThreadId { get; set; } = "";

        [CommandOption("--body <MODE>")]
        [Description("markdown, text, html, none or snippet")]
        [DefaultValue("markdown")]
        public string Body { get; set; } = "markdown";

        [CommandOption("--max-chars <N>")]
        [Description("Truncate each message body at this many characters (0 = no limit)")]
        [DefaultValue(8000)]
        public int MaxChars { get; set; } = 8000;

        [CommandOption("--max-messages <N>")]
        [Description("Return at most this many messages, most recent last")]
        [DefaultValue(20)]
        public int MaxMessages { get; set; } = 20;

        [CommandOption("--keep-quotes")]
        public bool KeepQuotes { get; set; }

        public override ValidationResult Validate()
        {
            if (!BodyModes.IsValid(Body))
                return ValidationResult.Error($"--body must be one of: {string.Join(", ", BodyModes.Names)}");
            if (MaxMessages < 1)
                return ValidationResult.Error("--max-messages must be at least 1.");
            return base.Validate();
        }
    }

    /// <summary>
    /// threads.get accepts full, metadata and minimal — but <em>not</em> raw, unlike messages.get.
    /// minimal is all we need here: the message ids, in order.
    /// </summary>
    public static string ThreadPath(string threadId) => $"threads/{threadId}?format=minimal";

    /// <summary>The per-message fetch, identical to the one <c>message get</c> makes.</summary>
    public static string MessagePath(string messageId) => $"messages/{messageId}?format=raw";

    protected override async Task<object?> RunAsync(GmailApiClient client, Settings settings, CancellationToken ct)
    {
        var mode = BodyModes.Parse(settings.Body);

        using var doc = await client.GetAsync(ThreadPath(settings.ThreadId), ct);
        var root = doc.RootElement;

        if (!root.TryGetProperty("messages", out var messages) || messages.ValueKind != JsonValueKind.Array)
            throw GmailException.NotFound($"Thread '{settings.ThreadId}' has no messages.");

        var all = messages.EnumerateArray()
            .Select(m => m.TryGetProperty("id", out var id) ? id.GetString() : null)
            .Where(id => !string.IsNullOrEmpty(id))
            .Select(id => id!)
            .ToList();
        var total = all.Count;

        // Keep the most recent: in a long thread the tail is what a reply needs. Select before
        // fetching so a long thread does not pull down more messages than will be shown.
        var selected = all.Count > settings.MaxMessages ? all[^settings.MaxMessages..] : all;

        var rendered = new List<object>();
        foreach (var id in selected)
        {
            // Sequentially: the bodies are the expensive part, and parallel fetches here buy
            // nothing but a shot at the rate limiter.
            using var message = await client.GetAsync(MessagePath(id), ct);
            var envelope = message.RootElement;

            var raw = envelope.TryGetProperty("raw", out var value) ? value.GetString() : null;
            if (string.IsNullOrEmpty(raw)) continue;

            var mime = MimeBuilder.FromRaw(raw);
            rendered.Add(MessageRenderer.Full(
                envelope, mime, mode, settings.MaxChars, settings.KeepQuotes, allHeaders: false));
        }

        var result = new Dictionary<string, object?>
        {
            ["account"] = client.Account.Name,
            ["threadId"] = settings.ThreadId,
            ["messageCount"] = total,
            ["returned"] = rendered.Count,
            ["messages"] = rendered
        };

        if (rendered.Count < total)
            result["note"] = $"Showing the {rendered.Count} most recent of {total} messages. Raise --max-messages for more.";

        return result;
    }
}

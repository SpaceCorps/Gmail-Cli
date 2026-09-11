namespace Gmail.Console.Tests;

using ThreadGet = Gmail.Console.Commands.Thread.GetCommand;

/// <summary>
/// Gmail allows a different set of format values on the two endpoints: users.messages.get takes
/// full, metadata, minimal and raw, but users.threads.get rejects raw with a 400 invalid_input.
/// `thread get` therefore lists the thread at a format threads.get accepts, and fetches each
/// message body through messages.get — the call that already works in `message get`.
/// </summary>
public class ThreadFetchPathTests
{
    [Fact]
    public void Thread_path_does_not_ask_for_raw()
    {
        var path = ThreadGet.ThreadPath("163d448a56f11f9c");

        Assert.DoesNotContain("format=raw", path);
        Assert.Equal("threads/163d448a56f11f9c?format=minimal", path);
    }

    [Fact]
    public void Message_path_asks_for_raw_so_a_body_can_be_parsed()
    {
        Assert.Equal("messages/163f345e576c5503?format=raw", ThreadGet.MessagePath("163f345e576c5503"));
    }
}

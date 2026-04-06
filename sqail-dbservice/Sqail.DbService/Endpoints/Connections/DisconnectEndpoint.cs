using FastEndpoints;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Connections;

public class DisconnectRequest
{
    public required string ConnectionId { get; init; }
}

public class DisconnectEndpoint(IConnectionManager connectionManager) : Endpoint<DisconnectRequest>
{
    public override void Configure()
    {
        Post("/api/connections/disconnect");
        AllowAnonymous();
    }

    public override async Task HandleAsync(DisconnectRequest req, CancellationToken ct)
    {
        await connectionManager.DisconnectAsync(req.ConnectionId);
        await Send.OkAsync(new { message = "Disconnected" });
    }
}

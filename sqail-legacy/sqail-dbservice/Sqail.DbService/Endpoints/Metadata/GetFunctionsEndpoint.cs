using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Metadata;

public class GetFunctionsEndpoint(IMetadataService metadataService) : EndpointWithoutRequest<List<FunctionInfo>>
{
    public override void Configure()
    {
        Get("/api/metadata/{connectionId}/functions");
    }

    public override async Task HandleAsync(CancellationToken ct)
    {
        var connectionId = Route<string>("connectionId")!;
        var metadata = await metadataService.GetMetadataAsync(connectionId);
        var functions = metadata.Schemas.SelectMany(s => s.Functions).ToList();
        await Send.OkAsync(functions);
    }
}

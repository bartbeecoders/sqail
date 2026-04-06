using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.StoredProcedures;

public class ExecuteSprocEndpoint(IQueryService queryService) : Endpoint<StoredProcedureRequest, QueryResult>
{
    public override void Configure()
    {
        Post("/api/sproc/execute");
        AllowAnonymous();
    }

    public override async Task HandleAsync(StoredProcedureRequest req, CancellationToken ct)
    {
        var result = await queryService.ExecuteStoredProcedureAsync(req);

        if (!result.Success)
        {
            HttpContext.Response.StatusCode = 400;
        }

        await Send.OkAsync(result);
    }
}

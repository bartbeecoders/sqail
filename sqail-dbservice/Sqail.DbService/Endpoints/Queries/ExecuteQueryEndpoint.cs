using FastEndpoints;
using Sqail.DbService.Models;
using Sqail.DbService.Services;

namespace Sqail.DbService.Endpoints.Queries;

public class ExecuteQueryEndpoint(IQueryService queryService) : Endpoint<QueryRequest, QueryResult>
{
    public override void Configure()
    {
        Post("/api/query/execute");
        AllowAnonymous();
    }

    public override async Task HandleAsync(QueryRequest req, CancellationToken ct)
    {
        var result = await queryService.ExecuteQueryAsync(req);

        if (!result.Success)
        {
            HttpContext.Response.StatusCode = 400;
        }

        await Send.OkAsync(result);
    }
}

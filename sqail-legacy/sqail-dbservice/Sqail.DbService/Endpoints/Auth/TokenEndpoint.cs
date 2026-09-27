using System.IdentityModel.Tokens.Jwt;
using System.Security.Claims;
using System.Security.Cryptography;
using System.Text;
using FastEndpoints;
using Microsoft.IdentityModel.Tokens;
using Sqail.DbService.Configuration;

namespace Sqail.DbService.Endpoints.Auth;

public record TokenRequest
{
    public required string ApiKey { get; init; }
}

public record TokenResponse
{
    public required string Token { get; init; }
    public required long ExpiresAt { get; init; }
}

public class TokenEndpoint(AppConfig config) : Endpoint<TokenRequest, TokenResponse>
{
    public override void Configure()
    {
        Post("/api/auth/token");
        AllowAnonymous();
    }

    public override async Task HandleAsync(TokenRequest req, CancellationToken ct)
    {
        if (!CryptographicOperations.FixedTimeEquals(
                Encoding.UTF8.GetBytes(req.ApiKey ?? ""),
                Encoding.UTF8.GetBytes(config.Jwt.ApiKey)))
        {
            HttpContext.Response.StatusCode = 401;
            await Send.StringAsync("Invalid API key", cancellation: ct);
            return;
        }

        var key = new SymmetricSecurityKey(Encoding.UTF8.GetBytes(config.Jwt.Secret));
        var creds = new SigningCredentials(key, SecurityAlgorithms.HmacSha256);
        var expires = DateTime.UtcNow.AddMinutes(config.Jwt.TokenLifetimeMinutes);
        var jwt = new JwtSecurityToken(
            issuer: config.Jwt.Issuer,
            audience: config.Jwt.Audience,
            claims: [new Claim("sub", "sqail-client")],
            expires: expires,
            signingCredentials: creds);

        var token = new JwtSecurityTokenHandler().WriteToken(jwt);
        await Send.OkAsync(new TokenResponse
        {
            Token = token,
            ExpiresAt = new DateTimeOffset(expires).ToUnixTimeSeconds(),
        }, cancellation: ct);
    }
}


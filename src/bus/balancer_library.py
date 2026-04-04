"""Pre-generated N-to-M balancer templates.

DO NOT EDIT MANUALLY. Regenerate with:
    uv run python scripts/generate_balancer_library.py

Shapes are oriented for vertical SOUTH flow: inputs at the top
(facing SOUTH), outputs at the bottom (facing SOUTH).
"""
from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class BalancerTemplateEntity:
    name: str
    x: int  # top-left tile (splitters span 2 tiles in their broad axis)
    y: int
    direction: int  # Factorio 1.0 direction (0=N, 2=E, 4=S, 6=W)
    io_type: str | None = None  # 'input'/'output' for underground-belt


@dataclass(frozen=True)
class BalancerTemplate:
    n_inputs: int
    n_outputs: int
    width: int
    height: int
    entities: tuple[BalancerTemplateEntity, ...]
    input_tiles: tuple[tuple[int, int], ...]  # (dx, dy) relative
    output_tiles: tuple[tuple[int, int], ...]
    source_blueprint: str  # for debugging / regeneration


BALANCER_TEMPLATES: dict[tuple[int, int], BalancerTemplate] = {
    (1, 2): BalancerTemplate(
        n_inputs=1,
        n_outputs=2,
        width=2,
        height=3,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=2, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=2, direction=4),
        ),
        input_tiles=((1, 0),),
        output_tiles=((0, 2), (1, 2)),
        source_blueprint="0eNqtkOtqwzAMhV+l6HcW4iyXtq8yQslFFEGiGFsdDcHvPs0L26BhbLB/R8dH5xNeoRtvaB2xwPmwAvUze1UvK3i6cjtGVxaLKoAEJ0gOwO0UZ29HEkEHQU3iAe/qmtAkW1QjX/VqvqLzNLP6+dEUdXGqq9pkVVnpG7KQEG7wOC0Xvk2d1mupJuzsNRHXV3gnZWmp9vKhwre7xLXs7ezkqcMxkgdy2G/LuUYfCfkeIf9PwvMewXwSTJqFvb/9VXfx8/Xmb9c3IbwB0pGkzg==",
    ),
    (1, 3): BalancerTemplate(
        n_inputs=1,
        n_outputs=3,
        width=3,
        height=9,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=2, y=0, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=4, direction=6),
            BalancerTemplateEntity(name="transport-belt", x=2, y=8, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=1, direction=4),
            BalancerTemplateEntity(name="splitter", x=1, y=2, direction=4),
            BalancerTemplateEntity(name="underground-belt", x=1, y=3, direction=4, io_type="input"),
            BalancerTemplateEntity(name="splitter", x=1, y=4, direction=6),
            BalancerTemplateEntity(name="underground-belt", x=1, y=6, direction=4, io_type="output"),
            BalancerTemplateEntity(name="splitter", x=1, y=7, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=8, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=1, direction=2),
            BalancerTemplateEntity(name="transport-belt", x=0, y=2, direction=0),
            BalancerTemplateEntity(name="transport-belt", x=0, y=3, direction=0),
            BalancerTemplateEntity(name="transport-belt", x=0, y=4, direction=0),
            BalancerTemplateEntity(name="transport-belt", x=0, y=5, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=6, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=7, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=8, direction=4),
        ),
        input_tiles=((2, 0),),
        output_tiles=((0, 8), (1, 8), (2, 8)),
        source_blueprint="0eNqtlOFugyAQx1+l4XPXACJqX2VpFm1JQ6JgEJcZw7vv5szWbCdZOz55HMf97nL3dyZNO6reaePJcTcTfbZmAOt5JoO+mrpdvH7qFRhEe9WR/Y6YulvOQ99q75UjAZzaXNQbeFk47ddQCPlOD85X5QZtDfh5yUQhqkIWjMpcwp0yXnutVvhyml7M2DWQHpJCRG8HiFiez+SDRA85uKdPK9zU5V1tht46/9SodiFftFPn9TGH0N8EjhFYSkKGEbKUBIERxGMEgRJyjFCm7EHG58D+TygwAr8h0IDt+J9yl/EZ/6h+BMm4q7PwxeuH85f2TD96gkIrDJofKA7daggfOKNYcpmiIzv6zZYYKvgizZAYj29xgh1jWXyNeRSBpxTxveX3VC1xRB5f3xQIGf9HpUAUuB4eQmyMt4yrIgWiiisgAYLTuA7uRJxCeAde3ooy",
    ),
    (1, 4): BalancerTemplate(
        n_inputs=1,
        n_outputs=4,
        width=4,
        height=4,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=3, y=3, direction=4),
            BalancerTemplateEntity(name="splitter", x=2, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=1, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=3, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=3, direction=4),
        ),
        input_tiles=((1, 0),),
        output_tiles=((0, 3), (1, 3), (2, 3), (3, 3)),
        source_blueprint="0eNqtkd1qwzAMhV+l+LoLsfPX9VVKKUkriiBRjK2WhuB3n2ZCV2gIG9mddHys71geVdPewDokVvvNqPDck5fqMCqPV6rbqPJgQQqFDJ3abhTVXey9bZEZnAoiIl3gIaoOx+1kFcvPeBHv4Dz2JLrZ6bzKP6uy0mlZlHIGxMgIEzx2w4luXSPjZag4bO/FEa+P6puUJYXIg1RpUoSXXOxq8rZ3/NFAG8kXdHCeLhuxvhPMHME8CTpJw9zLfzU7W06v16fP5wjpk2DWE4o5gn4hrNhPubyff0hfLf9utib9bjl99rf0xxC+AJD9GFk=",
    ),
    (2, 1): BalancerTemplate(
        n_inputs=2,
        n_outputs=1,
        width=2,
        height=3,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=1, direction=4),
        ),
        input_tiles=((0, 0), (1, 0)),
        output_tiles=((1, 2),),
        source_blueprint="0eNqtkdEKwjAMRX9F8qyyzrmpvyIimwYJbFlpM3GM/ruxDhQcvujbze3NPS0doKo7tI5YYDcbgE4te1X7ATxduKyjK71FFUCCDcxnwGUTZ29rEkEHQU3iM97UNeEwH6MaedWreUXnqWX1043Jimxb5IVJ8nWuZ8hCQjjC49QfuWsqrddSTdjWayKuD/AgJcu12v1Thbd7iSvZ29bJosI6ks/k8DQupxr9JKRThPSfhNX3N5jfCdkUwbwRkjD1ex/dhxDuAAqkzA==",
    ),
    (2, 2): BalancerTemplate(
        n_inputs=2,
        n_outputs=2,
        width=2,
        height=3,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=2, direction=4),
        ),
        input_tiles=((0, 0), (1, 0)),
        output_tiles=((0, 2), (1, 2)),
        source_blueprint="0eNqtketqwzAMhV+l6Hdb4jSXtq9SSslFDEGiGFstDcHvXs0LtLBQNrZ/0vHR+WR7grq7onXEAsfVBNQM7LU6TeDpg6suqjJa1AJIsIf1CrjqY+9tRyLoIKhI3OJdVRPO69mqlme8ijd0ngZWPd2brMwOZVGapMgLPUMWEsIZHrvxwte+1ngNVYcdvDri+ASfpGSbqzx+VeFlL3EVezs42dTYRXJLDpt5OFXrd0K6REj/k7B7fwfzd0K2RDAvhCQs/d6PsvP37/PL7c8hPAC7gMLO",
    ),
    (2, 4): BalancerTemplate(
        n_inputs=2,
        n_outputs=4,
        width=4,
        height=4,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=3, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=2, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=1, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=3, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=3, direction=4),
        ),
        input_tiles=((1, 0), (2, 0)),
        output_tiles=((0, 3), (1, 3), (2, 3), (3, 3)),
        source_blueprint="0eNqtkV1qwzAQhK8S9JwES/5LepUSgp0sZcFeC2lTaozu3q0wraFGtDhvq9Fov2E0qbZ7gHVIrF52k8LbQF6m10l5fKOmiyqPFmRQyNCr/U5R08eztx0yg1NBRKQ7fIiqw2U/W8Xys17Ed3AeBxLdnHRRF+e6qnVWlZXcATEywgyPp/FKj76V9bJUHHbw4ojPJ/VFyo+lyKNM2bEMi1zsGvJ2cHxooYvkOzq4zY+NWH8TzBoh+ybo7YR8jWAWhCysdfun3UW6nyekL9P9mO2Eao2gF4QN/dTpfp6Q/pT+3XxL+nM6ff6/9JcQPgEI0TZd",
    ),
    (3, 1): BalancerTemplate(
        n_inputs=3,
        n_outputs=1,
        width=3,
        height=9,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=2, y=0, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=5, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=7, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=8, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="underground-belt", x=1, y=2, direction=4, io_type="input"),
            BalancerTemplateEntity(name="transport-belt", x=1, y=3, direction=4),
            BalancerTemplateEntity(name="splitter", x=1, y=4, direction=4),
            BalancerTemplateEntity(name="underground-belt", x=1, y=5, direction=4, io_type="output"),
            BalancerTemplateEntity(name="splitter", x=1, y=6, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=7, direction=6),
            BalancerTemplateEntity(name="transport-belt", x=0, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=3, direction=2),
            BalancerTemplateEntity(name="transport-belt", x=0, y=4, direction=0),
            BalancerTemplateEntity(name="transport-belt", x=0, y=5, direction=0),
            BalancerTemplateEntity(name="transport-belt", x=0, y=6, direction=0),
            BalancerTemplateEntity(name="transport-belt", x=0, y=7, direction=0),
        ),
        input_tiles=((0, 0), (1, 0), (2, 0)),
        output_tiles=((2, 8),),
        source_blueprint="0eNqtlOFuhCAMx1/lwufbIpyi7lWWy6J35EKiYACXGcO7r3Nmu2xdMyOfbGvpD8q/zKztRjU4bQJ7OsxMX6zxYD3PzOubabolGqZBgcF0UD07Hphp+sX3Q6dDUI5FCGpzVW8Q5fF8XFMh5bs8BF+V89oaiIuK52Vel7LkmSwk/FMm6KDVCl+86cWMfQvloShkDNZDxrJ8Zh+k7LGA8PRpxbt9BdcYP1gXHlrVLeSrduqyLhaQ+psgMAJPSThhBJGSkGOEU0pCgRGKlASJEcqUhBIjVCkJFa1Wvp9Q01r6QRhhNN3NWfjiDPC/ZtwMY2AolGe0vhKci6ODnt8hsog9QP8rLmj17uiaHcPfbUMHXyY6U07Py6Y7yXFEQctZJLh2Sb++YleLSlq29P7xkhUt000tkTiipsWaACEyWpgpEJyW50bEOcZ3cM2oWg==",
    ),
    (4, 1): BalancerTemplate(
        n_inputs=4,
        n_outputs=1,
        width=4,
        height=4,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=3, y=0, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=2, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=1, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=1, direction=4),
        ),
        input_tiles=((0, 0), (1, 0), (2, 0), (3, 0)),
        output_tiles=((2, 3),),
        source_blueprint="0eNqtkd1qwzAMhV+l6LobsfPX9VVGGUkrhiBRjK2OheB3n+YFFlgIhexOPj4+30GeoO3u6DyxwPkwAV0HDjq9ThDonZsuqTI61AFIsIfjAbjp0zm4jkTQQ1SR+Iafqpp4Oc5WtfzGq/iBPtDAqtuTKeripa5qk1VlpXfIQkI4w9NpfON732q8hqrDDUEd6fkE36TsuVR5/Jniopf4hoMbvDy12CXyjTxe58dWrX8Jdptg9hPyNYJZELK4ttuHsou17Pw/25fb+7H7CdUawS4IO/ZTb7fP97c/bf9u/mj7S4xfCWoYTg==",
    ),
    (4, 2): BalancerTemplate(
        n_inputs=4,
        n_outputs=2,
        width=4,
        height=4,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=3, y=0, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=2, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=1, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=1, direction=4),
        ),
        input_tiles=((0, 0), (1, 0), (2, 0), (3, 0)),
        output_tiles=((1, 3), (2, 3)),
        source_blueprint="0eNqtkl1qwzAQhK8S9jktlvyX5ColBDtZyoK9FpJSYozu3q1qqKFGBJy31Wg03zJogra7o7HEHk67Ceg6sJPpYwJHn9x0UfWjQRmAPPaw3wE3fTw705H3aCGISHzDh6gqnPezVSx/8SJ+oXU0sOj6oIq6ONZVrbKqrOQO2ZMnnOHxNF743rcSL6HiMIMTR3w+wQ8pey9FHn+nsNjL24adGax/a7GL5BtZvM6PtVj/E3SaoLYT8jWCWhCysNbtU9nFWnb+yu3LdD96O6FaI+gFYUM/dbqfF2x/SPeTbycc0/8nf7afcwjfKm82Vg==",
    ),
    (4, 4): BalancerTemplate(
        n_inputs=4,
        n_outputs=4,
        width=4,
        height=10,
        entities=(
            BalancerTemplateEntity(name="transport-belt", x=3, y=0, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=3, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=3, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=3, y=4, direction=6),
            BalancerTemplateEntity(name="transport-belt", x=3, y=6, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=3, y=7, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=3, y=9, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=2, y=1, direction=4),
            BalancerTemplateEntity(name="underground-belt", x=2, y=3, direction=4, io_type="input"),
            BalancerTemplateEntity(name="transport-belt", x=2, y=4, direction=6),
            BalancerTemplateEntity(name="transport-belt", x=2, y=6, direction=2),
            BalancerTemplateEntity(name="underground-belt", x=2, y=7, direction=4, io_type="output"),
            BalancerTemplateEntity(name="splitter", x=2, y=8, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=2, y=9, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=1, y=2, direction=4),
            BalancerTemplateEntity(name="underground-belt", x=1, y=3, direction=4, io_type="input"),
            BalancerTemplateEntity(name="transport-belt", x=1, y=4, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=1, y=6, direction=2),
            BalancerTemplateEntity(name="underground-belt", x=1, y=7, direction=4, io_type="output"),
            BalancerTemplateEntity(name="transport-belt", x=1, y=9, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=0, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=1, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=2, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=3, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=4, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=5, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=6, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=7, direction=4),
            BalancerTemplateEntity(name="splitter", x=0, y=8, direction=4),
            BalancerTemplateEntity(name="transport-belt", x=0, y=9, direction=4),
        ),
        input_tiles=((0, 0), (1, 0), (2, 0), (3, 0)),
        output_tiles=((0, 9), (1, 9), (2, 9), (3, 9)),
        source_blueprint="0eNq1ld1ugzAMhV+lynU3EQcS2KtM1dSfqIoEAYUwrUK8+zxWbdWWeetwr0iM8WfMOWEUu3qwXXA+iofVKNy+9T2uHkfRu6Pf1nM0njqLC+GibcR6Jfy2mfd9V7sYbRATBp0/2BeMymmzPqdiymd5DD7b0LvWYxxKmZu8MtrITBca71kfXXT2DJ93pyc/NDssj0Uxo2t7zJgfH8UbKbsvMHx6X00XfcWw9X3Xhni3s/VMPrhg9+eHAVO/EyBFAE6CShEUJyFPEfL/EfIkoUgRNOc76BTBcBJMilBxEkparXI5oUoR5AUhm1I+/VNtmdFC/dL+gL4Px9DiNf0CuP84QHw3RJGmSlq8crl4JdDqpRHpkoqW64JZtUP8eVhJp5dM37+gDcIgX6lphwADwtAHOiwaUUlbBG5jkYq2CMPUIKMtAtdbBCRtEbiNRQBoFXMMS9EqVgyInD7o1RIVQ0FbhKN/TRuFA2FoV3AgknYvmL5CRVuOoX+V0RbkQEj6j7RoROoXL1/Z/2aaXgF1hPVJ",
    ),
}

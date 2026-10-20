---
title: A Short Title That Says What Was Found
author: Ada Example and Ben Sample
cite-style: ieee
default-font: CMU Serif
default-size: 10.5
page:
  size: A4
  margins: {top: 1.1in, bottom: 1.2in, left: 1.4in, right: 1.4in}
  footer: {center: "{page}"}
styles:
  normal: {name: Normal, align: justify, first-line: 15pt, space-after: 0, line-height: 1.25}
  title: {name: Title, next: subtitle, align: center, size: 17.3, space-before: 8, space-after: 12, first-line: 0, line-height: 1.15}
  subtitle: {name: Subtitle, next: normal, align: center, size: 12, space-after: 3, first-line: 0}
  heading1: {name: Heading 1, next: normal, outline-level: 1, size: 14.4, bold: true, align: left, first-line: 0, space-before: 16, space-after: 8, keep-with-next: true, line-height: 1.15}
  heading2: {name: Heading 2, next: normal, outline-level: 2, size: 12, bold: true, align: left, first-line: 0, space-before: 12, space-after: 6, keep-with-next: true, line-height: 1.15}
  abstract-heading: {name: Abstract heading, next: abstract, size: 9.5, bold: true, align: center, first-line: 0, space-before: 14, space-after: 4}
  abstract: {name: Abstract, size: 9.5, align: justify, first-line: 0, indent-left: 30, indent-right: 30, space-after: 0}
  caption: {name: Caption, size: 9.5, align: center, first-line: 0, space-before: 4, space-after: 12}
  equation: {name: Equation, next: first, align: center, first-line: 0, space-before: 8, space-after: 8}
  footer: {name: Footer, align: center, size: 10.5}
  first: {name: First paragraph, next: normal, align: justify, first-line: 0, space-after: 0, line-height: 1.25}
references:
  - id: knuth1984
    type: book
    author: [{family: Knuth, given: Donald E.}]
    title: The TeXbook
    publisher: Addison-Wesley
    issued: {date-parts: [[1984]]}
  - id: lamport1994
    type: book
    author: [{family: Lamport, given: Leslie}]
    title: "LaTeX: A Document Preparation System"
    publisher: Addison-Wesley
    issued: {date-parts: [[1994]]}
---

::: {custom-style=Title}
A Short Title That Says What Was Found
:::

::: {custom-style=Subtitle}
Ada Example · Ben Sample
:::

::: {custom-style=Subtitle}
Department, University · email@example.org
:::

::: {custom-style=Subtitle}
[date]{field=date}
:::

::: {custom-style="Abstract heading"}
Abstract
:::

::: {custom-style=Abstract}
Say in one paragraph what the paper is about: the question, how it was studied, what was found and why it matters. Keep it to about 150 words, with no citations or equations, so it can be read on its own.
:::

# 1 Introduction

::: {custom-style="First paragraph"}
Open with the problem and why it matters. As in LaTeX's article class, the first paragraph after a heading starts flush left; the paragraphs after it are indented and follow on with no space between them [@knuth1984].
:::

State what is already known, what is missing, and what this paper adds. End the introduction with a sentence saying how the rest of the paper is organised.

# 2 Method

::: {custom-style="First paragraph"}
Describe what was done in enough detail for someone else to repeat it. Equations sit on a line of their own:
:::

::: {custom-style=Equation}
$$E = \frac{1}{2} m v^2 + m g h$$
:::

::: {custom-style="First paragraph"}
where $m$ is the mass, $v$ the speed and $h$ the height. Double-click an equation to edit it.
:::

## 2.1 Data

::: {custom-style="First paragraph"}
Where the data came from and how it was prepared.
:::

# 3 Results

::: {custom-style="First paragraph"}
Report what was found, in the order of the questions in the introduction. Put numbers in tables and refer to them in the text (Table 1).
:::

::: {.table border=none width=70 align=center header-fill=#ffffff cells="1.1 top=1#000000 bottom=0.5#000000; 1.2 top=1#000000 bottom=0.5#000000; 1.3 top=1#000000 bottom=0.5#000000; 3.1 bottom=1#000000; 3.2 bottom=1#000000; 3.3 bottom=1#000000"}
| Condition | Mean | SD |
|-----------|-----:|---:|
| Control   | 4.2  | 0.8 |
| Treatment | 5.1  | 0.7 |
:::

::: {custom-style=Caption}
Table 1: Results for each condition.
:::

# 4 Discussion

::: {custom-style="First paragraph"}
What the results mean, how they compare with earlier work [@lamport1994], and what their limits are.
:::

# 5 Conclusion

::: {custom-style="First paragraph"}
The main finding in two or three sentences, and what should be done next.
:::

# References

::: {.bibliography levels=3}
:::

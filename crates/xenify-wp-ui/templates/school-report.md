---
title: The Title of My Project
author: Your Name
page:
  size: A4
  vertical-align: center
styles:
  title: {name: Title, next: subtitle, align: center, size: 30, bold: true, space-after: 10, line-height: 1.1}
  subtitle: {name: Subtitle, next: normal, align: center, size: 15, color: "#555555", space-after: 36}
  cover: {name: Cover details, align: center, size: 13, space-after: 6}
  heading1: {name: Heading 1, next: normal, outline-level: 1, size: 18, bold: true, color: "#1f4e79", space-before: 20, space-after: 8, keep-with-next: true, line-height: 1.15}
  contents-heading: {name: Contents heading, next: normal, size: 18, bold: true, color: "#1f4e79", space-after: 8}
  heading2: {name: Heading 2, next: normal, outline-level: 2, size: 14, bold: true, color: "#2e75b6", space-before: 14, space-after: 6, keep-with-next: true, line-height: 1.15}
---

::: {custom-style=Title}
The Title of My Project
:::

::: {custom-style=Subtitle}
A question or a one-line description of what the project is about
:::

::: {custom-style="Cover details"}
Your Name
:::

::: {custom-style="Cover details"}
Class · Teacher's name
:::

::: {custom-style="Cover details"}
School name
:::

::: {custom-style="Cover details"}
[date]{field=date}
:::

::: {.section start=new-page page="{\"size\":\"A4\",\"start-number\":1,\"header\":{\"right\":\"{title}\"},\"footer\":{\"center\":\"Page {page}\"}}"}
::: {custom-style="Contents heading"}
Contents
:::

::: {.toc levels=2}
:::

# Introduction

What is the project about, and why did you choose it? Say what question you set out to answer, and what you expected to find.

# Background

What did you need to know before you started? Explain the ideas and words a reader needs, and say where you found them.

# Method

How did you go about it, step by step? Say what you used and what you did, so someone else could do it again.

1. First step
2. Second step
3. Third step

# Results

What did you find? Use a table or a picture where it helps, and say in words what it shows.

| What I measured | First try | Second try |
|-----------------|----------:|-----------:|
| Example         | 10        | 12         |

# Conclusion

What is the answer to your question? Was it what you expected? What would you do differently, or look at next?

# References

- Author, *Title of the book or web page*, year, and where you found it.
- Another source you used.
:::

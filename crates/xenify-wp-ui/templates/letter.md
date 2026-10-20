---
title: Letter
merge:
  file-name: "letter-{{last}}"
page:
  size: A4
  margins: {top: 72, bottom: 72, left: 72, right: 72}
  footer: {center: "Your Company · 1 High Street · Leeds LS1 1AA · hello@example.com"}
---

::: {#frame1 .frame x=340 y=60 width=190 height=90 wrap=around padding=0}
**Your Company**

1 High Street

Leeds LS1 1AA
:::

::: {#frame2 .frame x=72 y=160 width=240 height=90 wrap=top-bottom padding=0}
{{first|Customer}} {{last}}

{{street}}

{{city}} {{postcode}}
:::

::: {align=right}
{{date|30 September 2026}}
:::

Dear {{first|Customer}},

Thank you for your recent order. Write the body of your letter here: fields in double braces, like {{first}}, are filled in from your data when you use Mail merge.

Kind regards,

 

Your Name

Your Title

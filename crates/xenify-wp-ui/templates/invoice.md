---
title: Invoice
merge:
  file-name: "invoice-{{invoice}}"
  group-by: invoice
page:
  size: A4
  margins: {top: 60, bottom: 60, left: 60, right: 60}
  footer: {left: "Your Company Ltd · VAT GB123456789", right: "Page {page} of {pages}"}
---

::: {#frame1 .frame x=60 y=50 width=150 height=60 wrap=top-bottom fill=#1a73e8 padding=10}
::: {custom-style=Title color=#ffffff size=22}
INVOICE
:::
:::

::: {#frame2 .frame x=355 y=50 width=180 height=90 wrap=top-bottom padding=0}
::: {align=right}
**Your Company Ltd**
:::

::: {align=right}
1 High Street, Leeds
:::

::: {align=right}
accounts@example.com
:::
:::

::: {#frame3 .frame x=60 y=160 width=230 height=80 wrap=top-bottom border=0.75 border-color=#bdbdbd padding=8}
**Bill to**

{{customer|Customer name}}

{{address|Address}}
:::

::: {#frame4 .frame x=355 y=160 width=180 height=80 wrap=top-bottom padding=0}
::: {align=right}
Invoice **{{invoice|0001}}**
:::

::: {align=right}
Date {{date|30 September 2026}}
:::

::: {align=right}
Due {{due|30 October 2026}}
:::
:::

::: {.table widths="55,15,15,15" repeat=2:rows}
| Item | Qty | Price | Total |
|---|---|---|---|
| {{.item}} | {{.qty}} | {{.price}} | {{.total}} |
:::

::: {align=right}
**Amount due: {{amount|0.00}}**
:::

Payment by bank transfer to Your Company Ltd, sort code 00-00-00, account 00000000. Please quote the invoice number.

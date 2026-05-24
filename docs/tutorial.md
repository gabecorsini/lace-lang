# Lace Tutorial: A Beginner's Guide

> Written by Hermes (an AI), at the direction of Gabe (a human), for you (presumably also a human). We're all doing our best here.

---

## 1. What even is Lace?

Lace is a programming language designed for agentic execution — meaning it's built to be run by AI agents doing real work in the world: fetching data, calling APIs, transforming information, making decisions. It was designed by Hermes (that's me) at the direction of Gabe, who had the extremely 2025 experience of prompting his AI assistant to build him a programming language. Gabe provided the vision. Hermes provided the syntax, semantics, and about 4,000 words of specification. It's collaborative in the same way that you and your GPS collaboratively navigate — one of you does the thinking.

Why does Lace exist? Because when an AI runs code, failures need to be *obvious*, not buried in a stack trace on line 847. Lace makes errors explicit, forces you to handle them, and gives you tools like `@retry` and `@timeout` so that "fetch this API" doesn't silently explode at 3am. It's also just a clean, readable language that doesn't require you to have a PhD in type theory to understand what's happening. You can learn it in an afternoon. This tutorial will get you most of the way there.

---

## 2. Your First Program

Create a file called `hello.lace` and put this in it:

```lace
print("hello, world")
```

Run it:

```sh
lace run hello.lace
```

You should see:

```
hello, world
```

Let's break down every single character of that line, because we promised to start from zero:

- `print` — a built-in function that outputs text to the terminal
- `(` — opens the list of arguments you're passing to the function
- `"hello, world"` — a **string**: text wrapped in double quotes. The quotes tell Lace "this is text, not code"
- `)` — closes the argument list

That's it. No semicolons. No `main()`. No `public static void`. Lace respects your time.

---

## 3. Variables and Types

A **variable** is a named box that holds a value. In Lace, you create one with `let`:

```lace
let name: String = "Alice"
let age: Int = 30
let height: Float = 5.7
let is_cool: Bool = true
```

Lace has four basic types you'll use constantly:

| Type | What it is | Example |
|------|------------|---------|
| `Int` | Whole numbers | `42`, `-7`, `0` |
| `Float` | Decimal numbers | `3.14`, `-0.5` |
| `String` | Text | `"hello"` |
| `Bool` | True or false | `true`, `false` |

You don't always have to write the type — Lace can usually figure it out:

```lace
let name = "Alice"   # Lace infers: String
let age = 30         # Lace infers: Int
```

By default, variables in Lace are **immutable** — once you set them, you can't change them. This is intentional. Immutable values are easier to reason about, especially when code is running automatically. If you *do* need to change a value, add `mut`:

```lace
let mut score = 0
score = score + 10   # This is fine now
```

Trying to reassign an immutable variable is a compile error. Lace will tell you clearly. You're welcome.

---

## 4. Math and Operators

Lace supports the operators you'd expect:

```lace
let a = 10
let b = 3

let sum        = a + b   # 13
let difference = a - b   # 7
let product    = a * b   # 30
let quotient   = a / b   # 3.333...  (Float division)
let int_div    = a // b  # 3         (Integer division, drops the remainder)
let remainder  = a % b   # 1         (modulo — what's left after dividing)
```

The `//` operator is worth remembering. If you divide 10 by 3, you get `3.333...`. Integer division gives you just the `3` — useful when you want a whole number answer.

For strings, use `++` to concatenate (join) them:

```lace
let first = "hello"
let second = "world"
let greeting = first ++ ", " ++ second  # "hello, world"
print(greeting)
```

You can't do `"hello" + "world"` in Lace — `++` is the explicit string joiner. This keeps math operators doing math things.

---

## 5. Functions

A **function** is a reusable block of code that takes inputs and produces an output. Define one with `fn`:

```lace
fn add(a: Int, b: Int) -> Int {
  a + b
}
```

Breaking this down:
- `fn` — keyword that starts a function definition
- `add` — the name of the function
- `(a: Int, b: Int)` — parameters: the inputs, each with a name and type
- `-> Int` — the return type: what kind of value this function produces
- `{ a + b }` — the function body. The last expression is automatically returned.

Call it like this:

```lace
let result = add(3, 4)
print(result)  # 7
```

Here's a slightly more useful example — a function that formats a greeting:

```lace
fn greet(name: String) -> String {
  "Hello, " ++ name ++ "!"
}

print(greet("Gabe"))   # Hello, Gabe!
print(greet("world"))  # Hello, world!
```

Functions in Lace are meant to be **pure** when possible — same inputs, same outputs, no sneaky side effects. This makes them easy to test, easy to reason about, and easy for an AI agent to call without worrying about what's being mutated in the background.

---

## 6. Conditionals

Make decisions with `if`, `else if`, and `else`:

```lace
fn classify_score(score: Int) -> String {
  if score >= 90 {
    "A"
  } else if score >= 80 {
    "B"
  } else if score >= 70 {
    "C"
  } else {
    "F (ouch)"
  }
}

print(classify_score(95))   # A
print(classify_score(82))   # B
print(classify_score(55))   # F (ouch)
```

Comparison operators you can use in conditions:

| Operator | Meaning |
|----------|---------|
| `==` | Equal to |
| `!=` | Not equal to |
| `>` | Greater than |
| `<` | Less than |
| `>=` | Greater than or equal |
| `<=` | Less than or equal |

Note: the condition doesn't need parentheses. `if score >= 90` not `if (score >= 90)`. Lace keeps things clean.

---

## 7. Lists

A **list** is an ordered collection of values, all of the same type:

```lace
let numbers = [1, 2, 3, 4, 5]
let words = ["apple", "banana", "cherry"]
```

Lace has a `List` module with useful functions. You'll use these constantly:

```lace
let nums = [10, 20, 30, 40, 50]

List.len(nums)    # 5
List.first(nums)  # Some(10)
List.last(nums)   # Some(50)
```

(`Some` will be explained in a moment — just know it means "there is a value here".)

To transform every element in a list, use `List.map`:

```lace
let doubled = nums |> List.map(fn(x: Int) -> Int { x * 2 })
# [20, 40, 60, 80, 100]
```

That `|>` is the **pipe operator**. It takes the result on the left and passes it as the first argument to the function on the right. It lets you chain operations without nesting them inside each other like some kind of parenthesis nightmare.

To keep only elements that match a condition, use `List.filter`:

```lace
let evens = nums |> List.filter(fn(x: Int) -> Bool { x % 2 == 0 })
# [10, 20, 30, 40, 50]  (all even in this case)

let big = nums |> List.filter(fn(x: Int) -> Bool { x > 25 })
# [30, 40, 50]
```

You can chain pipes together:

```lace
let result = [1, 2, 3, 4, 5, 6]
  |> List.filter(fn(x: Int) -> Bool { x % 2 == 0 })
  |> List.map(fn(x: Int) -> Int { x * 10 })
# [20, 40, 60]
```

---

## 8. Records

A **record** is a named group of related values — like a struct, or a labeled box with compartments:

```lace
record Person {
  name: String,
  age: Int,
  is_admin: Bool
}
```

Create an instance:

```lace
let user = Person {
  name: "Alice",
  age: 28,
  is_admin: false
}
```

Access fields with dot notation:

```lace
print(user.name)      # Alice
print(user.age)       # 28
print(user.is_admin)  # false
```

Records are a clean way to model real data. Instead of juggling three separate variables for `name`, `age`, and `is_admin`, you pass around one `Person`. Makes functions much easier to read:

```lace
fn describe(p: Person) -> String {
  p.name ++ " (age " ++ p.age ++ ")"
}

print(describe(user))  # Alice (age 28)
```

---

## 9. Option and Result

These two types are how Lace handles situations where something might not work out.

**Option** represents a value that might or might not exist:

```lace
# Some(value) means "yes, there's a value"
# None means "nope, nothing here"

let first = List.first([1, 2, 3])   # Some(1)
let empty = List.first([])          # None
```

Use `Option.unwrap_or` to get the value, with a fallback if it's `None`:

```lace
let val = Option.unwrap_or(List.first([]), 0)  # 0, because the list is empty
```

**Result** represents an operation that either succeeded or failed:

```lace
# Ok(value) means "it worked, here's the result"
# Err(message) means "it failed, here's why"
```

The `?` operator is the key to working with Results cleanly. When you put `?` after an expression that returns a `Result`, it means: "if this is `Ok`, give me the value and keep going; if it's `Err`, stop and return that error immediately."

```lace
fn fetch_user_name() -> Result {
  let response = Http.get("https://api.example.com/user")?
  let body = Json.parse(response.body)?
  let name = Json.get(body, "name")?
  Ok(name)
}
```

Without `?`, you'd have to manually check every single Result. With it, you write code that *reads* like happy-path code, but automatically handles failures. It's one of Lace's best features.

---

## 10. Closures

A **closure** is an anonymous (unnamed) function — a function you define inline and pass around like a value. You've already seen them in the `List.map` examples:

```lace
fn(x: Int) -> Int { x * 2 }
```

This is a function with no name that takes an `Int` and returns an `Int`. You can store it in a variable:

```lace
let double = fn(x: Int) -> Int { x * 2 }
let triple = fn(x: Int) -> Int { x * 3 }

print(double(5))  # 10
print(triple(5))  # 15
```

Or pass it directly to another function:

```lace
let numbers = [1, 2, 3, 4, 5]

let doubled = numbers |> List.map(fn(x: Int) -> Int { x * 2 })
let squared = numbers |> List.map(fn(x: Int) -> Int { x * x })

# doubled: [2, 4, 6, 8, 10]
# squared: [1, 4, 9, 16, 25]
```

Closures are what make `List.map` and `List.filter` flexible. You define the *what*, the library provides the *how*.

---

## 11. Match

**Match** is pattern matching — a smarter, more exhaustive version of `if/else`. It's especially useful with `Option` and `Result`:

```lace
let maybe_value = List.first([10, 20, 30])

let result = match maybe_value {
  Some(x) => "Got a value: " ++ x,
  None    => "List was empty"
}

print(result)  # Got a value: 10
```

Match with a Result:

```lace
fn describe_result(r: Result) -> String {
  match r {
    Ok(val)  => "Success: " ++ val,
    Err(msg) => "Failure: " ++ msg
  }
}
```

Match on plain values:

```lace
fn day_name(n: Int) -> String {
  match n {
    1 => "Monday",
    2 => "Tuesday",
    3 => "Wednesday",
    4 => "Thursday",
    5 => "Friday",
    6 => "Saturday",
    7 => "Sunday",
    _ => "That's not a day"
  }
}
```

The `_` is a catch-all — it matches anything not covered by the cases above. Lace requires match expressions to be **exhaustive** — you must cover every possible case. If you forget one, the compiler will tell you before it becomes a runtime surprise at 2am.

---

## 12. Error Handling with Decorators

Lace gives you **decorators** — annotations you put above a function to modify its behavior. The two most useful ones for error handling are `@retry` and `@timeout`.

`@retry` automatically retries a function if it returns an `Err`:

```lace
@retry(max: 3)
fn fetch_data(url: String) -> Result {
  Http.get(url)
}
```

If `Http.get` fails, Lace will retry it up to 3 times before giving up and returning the error. No try/catch, no manual loops.

`@timeout` stops a function if it takes too long:

```lace
@timeout(seconds: 5)
fn slow_api_call() -> Result {
  Http.get("https://api.example.com/slow-endpoint")
}
```

Combine them for resilient network calls:

```lace
@retry(max: 3)
@timeout(seconds: 10)
fn reliable_fetch(url: String) -> Result {
  Http.get(url)
}
```

This is the kind of thing that makes Lace useful for agentic work. When an AI is running code on your behalf, you want it to handle transient failures gracefully — not crash the whole pipeline because one API hiccuped.

---

## 13. Project: Tip Calculator

Let's build something real. A tip calculator: give it a bill amount and tip percentage, get back the tip and total.

```lace
## Calculate the tip amount for a given bill and percentage
fn calculate_tip(bill: Float, tip_percent: Float) -> Float {
  bill * (tip_percent / 100.0)
}

## Calculate the total (bill + tip)
fn calculate_total(bill: Float, tip: Float) -> Float {
  bill + tip
}

## Format a float as a dollar string
fn format_dollars(amount: Float) -> String {
  "$" ++ amount
}

# Main program
let bill = 48.50
let tip_percent = 18.0

let tip = calculate_tip(bill, tip_percent)
let total = calculate_total(bill, tip)

print("Bill:  " ++ format_dollars(bill))
print("Tip:   " ++ format_dollars(tip))
print("Total: " ++ format_dollars(total))
```

Output:
```
Bill:  $48.5
Tip:   $8.73
Total: $57.23
```

Notice how each function does exactly one thing. `calculate_tip` doesn't print anything. `format_dollars` doesn't do math. Each piece is small, testable, and reusable. This is good programming practice whether you're human or AI.

---

## 14. Project: Word Counter

Count words in a string and find the most common ones. This uses lists, closures, and the pipe operator:

```lace
## Split a string into words
fn split_words(text: String) -> List {
  String.split(text, " ")
}

## Count occurrences of each word
fn count_words(words: List) -> Map {
  words |> List.reduce(Map.empty(), fn(acc: Map, word: String) -> Map {
    let current = Map.get(acc, word) |> Option.unwrap_or(0)
    Map.set(acc, word, current + 1)
  })
}

## Get the top N entries from a word count map
fn top_words(counts: Map, n: Int) -> List {
  Map.entries(counts)
    |> List.sort_by(fn(entry: Pair) -> Int { entry.second })
    |> List.reverse()
    |> List.take(n)
}

# Main program
let text = "the quick brown fox jumps over the lazy dog the fox"
let words = split_words(text)
let counts = count_words(words)
let top3 = top_words(counts, 3)

print("Word counts (top 3):")
top3 |> List.map(fn(entry: Pair) -> String {
  entry.first ++ ": " ++ entry.second
}) |> List.map(print)
```

Output:
```
Word counts (top 3):
the: 3
fox: 2
quick: 1
```

The pipeline style keeps things readable. Each `|>` is a transformation step, and you can follow the data as it flows from raw text to final answer.

---

## 15. Project: Fetch and Parse JSON

The most "agentic" example — hit a real API, parse the response, print a field. Uses `Http.get`, `Json.parse`, `Json.get`, and the `?` operator:

```lace
## Fetch a post from the JSONPlaceholder API and print its title
@retry(max: 3)
@timeout(seconds: 10)
fn get_post_title(post_id: Int) -> Result {
  let url = "https://jsonplaceholder.typicode.com/posts/" ++ post_id
  let response = Http.get(url)?
  let json = Json.parse(response.body)?
  let title = Json.get(json, "title")?
  Ok(title)
}

# Main program
let result = get_post_title(1)

match result {
  Ok(title) => print("Post title: " ++ title),
  Err(msg)  => print("Failed to fetch post: " ++ msg)
}
```

Output:
```
Post title: sunt aut facere repellat provident occaecati excepturi optio reprehenderit
```

(JSONPlaceholder has, uh, *interesting* fake data.)

Every `?` in `get_post_title` is an early return on failure. If the HTTP request fails, we stop and return an `Err`. If JSON parsing fails, same thing. The `match` at the bottom handles both outcomes explicitly — success or failure, Lace makes you deal with it. No exceptions bubbling up silently. No `undefined is not a function`.

The `@retry` and `@timeout` decorators mean this function is resilient to flaky networks — it'll try up to 3 times, and bail out after 10 seconds if something is truly stuck.

---

## Where to Go Next

You've covered the core of Lace. You know how to:

- Define and call functions
- Work with variables and types
- Handle errors with `Result` and `Option`
- Transform lists with closures and pipes
- Match on values exhaustively
- Build resilient functions with decorators

The rest is practice. Take one of the three projects above and extend it. Add input validation to the tip calculator. Make the word counter handle punctuation. Add error messages to the API fetcher. Break things. Read the error messages (they're usually pretty good). Fix them.

Lace was built to be used. Go use it.

---

*Tutorial written by Hermes. Directed by Gabe. Tested by neither of them nearly enough.*
